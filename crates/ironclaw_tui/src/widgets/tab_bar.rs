//! Tab bar widget: shows styled tabs with icons, active indicator, and notification badges.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::{TopTabBarMode, TuiSlot};
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

const TAB_SPECS: [(ActiveTab, &str, &str); 7] = [
    (ActiveTab::Conversation, "\u{25E6}", "Chat"),
    (ActiveTab::Workspace, "\u{25C8}", "Workspace"),
    (ActiveTab::Projects, "\u{25A0}", "Projects"),
    (ActiveTab::Jobs, "\u{2692}", "Jobs"),
    (ActiveTab::Missions, "\u{25B6}", "Missions"),
    (ActiveTab::Logs, "\u{25B8}", "Logs"),
    (ActiveTab::Settings, "\u{2699}", "Settings"),
];

fn tab_badge_count(tab: ActiveTab, state: &AppState) -> usize {
    match tab {
        ActiveTab::Conversation => state.messages.len(),
        ActiveTab::Workspace => state.memory_count,
        ActiveTab::Projects => state.engine_threads.len(),
        ActiveTab::Jobs => state.jobs.len(),
        ActiveTab::Missions => state
            .engine_threads
            .iter()
            .filter(|thread| thread.thread_type.eq_ignore_ascii_case("mission"))
            .count(),
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
    mode: TopTabBarMode,
    show_badges: bool,
}

impl TabBarWidget {
    pub fn new(theme: Theme, mode: TopTabBarMode, show_badges: bool) -> Self {
        Self {
            theme,
            mode,
            show_badges,
        }
    }

    pub fn render_with_mode(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &AppState,
        mode: TopTabBarMode,
    ) {
        if area.height == 0 || area.width < 20 || mode == TopTabBarMode::Hidden {
            return;
        }

        if matches!(mode, TopTabBarMode::Compact | TopTabBarMode::Auto) {
            self.render_compact(area, buf, state);
            return;
        }

        let mut spans: Vec<Span> = Vec::with_capacity(TAB_SPECS.len() * 5 + 2);
        spans.push(Span::styled(" ", self.theme.tab_bar_style()));
        let baseline_style = self.theme.chrome_border_style();

        for (i, (tab_id, icon, label)) in TAB_SPECS.iter().enumerate() {
            let badge_count = tab_badge_count(*tab_id, state);
            let is_active = state.active_tab == *tab_id;

            if i > 0 {
                spans.push(Span::styled("  ", baseline_style));
            }

            if is_active {
                let active_style = self
                    .theme
                    .tab_active_style()
                    .add_modifier(Modifier::UNDERLINED);
                spans.push(Span::styled(format!(" {icon} "), active_style));
                spans.push(Span::styled(*label, active_style));
            } else {
                let inactive_style = self.theme.tab_inactive_style();
                spans.push(Span::styled(format!("{icon} "), inactive_style));
                spans.push(Span::styled(*label, inactive_style));
            }

            if self.show_badges && !is_active && badge_count > 0 {
                let badge_text = if badge_count > 99 {
                    " 99+".to_string()
                } else {
                    format!(" {badge_count}")
                };
                spans.push(Span::styled(
                    badge_text,
                    Style::default()
                        .bg(self.theme.tab_bar_bg.to_color())
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ));
            }
        }

        let right_indicator = if state.is_streaming {
            let spinner_frame = state.tick_count % 4;
            let dots = match spinner_frame {
                0 => "\u{2022}  ",
                1 => " \u{2022} ",
                2 => "  \u{2022}",
                _ => " \u{2022} ",
            };
            Some(Span::styled(format!("  {dots}"), self.theme.accent_style()))
        } else if !state.active_tools.is_empty() {
            Some(Span::styled(
                format!("  \u{26A1}{}", state.active_tools.len()),
                self.theme.warning_style(),
            ))
        } else {
            None
        };

        if let Some(indicator) = right_indicator {
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let indicator_len = indicator.content.chars().count();
            let fill = (area.width as usize).saturating_sub(used + indicator_len + 1);
            if fill > 0 {
                spans.push(Span::raw(" ".repeat(fill)));
            }
            spans.push(indicator);
        }

        ratatui::widgets::Paragraph::new(Line::from(spans))
            .style(self.theme.tab_bar_style())
            .render(area, buf);
    }

    fn render_compact(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let mut spans: Vec<Span> = vec![
            Span::styled(" ", self.theme.tab_bar_style()),
            Span::styled("control room", self.theme.dim_style()),
            Span::styled("  /  ", self.theme.dim_style()),
            Span::styled(state.active_tab.title(), self.theme.tab_active_style()),
        ];

        let indicator = if state.is_streaming {
            Span::styled("  streaming", self.theme.accent_style())
        } else if !state.active_tools.is_empty() {
            Span::styled(
                format!("  ⚡{} active", state.active_tools.len()),
                self.theme.warning_style(),
            )
        } else {
            Span::styled("  use left rail to navigate", self.theme.dim_style())
        };

        let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        let indicator_len = indicator.content.chars().count();
        let fill = (area.width as usize).saturating_sub(used + indicator_len + 1);
        if fill > 0 {
            spans.push(Span::raw(" ".repeat(fill)));
        }
        spans.push(indicator);

        ratatui::widgets::Paragraph::new(Line::from(spans))
            .style(self.theme.tab_bar_style())
            .render(area, buf);
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
        self.render_with_mode(area, buf, state, self.mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_renders_without_panic() {
        let theme = Theme::dark();
        let widget = TabBarWidget::new(theme, TopTabBarMode::Full, true);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
    }

    #[test]
    fn tab_bar_too_narrow_skips_render() {
        let theme = Theme::dark();
        let widget = TabBarWidget::new(theme, TopTabBarMode::Full, true);
        let state = AppState::default();
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
        // Should not panic and should produce empty buffer
    }

    #[test]
    fn tab_bar_exposes_webgui_surface_order() {
        let tabs: Vec<_> = TAB_SPECS.iter().map(|(tab, _, _)| *tab).collect();
        assert_eq!(
            tabs,
            vec![
                ActiveTab::Conversation,
                ActiveTab::Workspace,
                ActiveTab::Projects,
                ActiveTab::Jobs,
                ActiveTab::Missions,
                ActiveTab::Logs,
                ActiveTab::Settings,
            ]
        );
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
            active_tab: ActiveTab::Projects,
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

        let hit_areas = tab_hit_areas(Rect::new(0, 0, 120, 1), &state);
        let (_, missions_range) = hit_areas
            .iter()
            .find(|(tab, _)| *tab == ActiveTab::Missions)
            .expect("missions range");
        let (_, logs_range) = hit_areas
            .iter()
            .find(|(tab, _)| *tab == ActiveTab::Logs)
            .expect("logs range");
        let (_, settings_range) = hit_areas
            .iter()
            .find(|(tab, _)| *tab == ActiveTab::Settings)
            .expect("settings range");

        assert!(missions_range.end > missions_range.start);
        assert!(logs_range.start >= missions_range.end);
        assert!(settings_range.start >= logs_range.end);
    }
}
