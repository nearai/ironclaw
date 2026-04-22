//! Left navigation rail for the control-room shell.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

const NAV_ITEMS: [(ActiveTab, &str, &str); 7] = [
    (ActiveTab::Conversation, "◦", "Chat"),
    (ActiveTab::Workspace, "◈", "Workspace"),
    (ActiveTab::Projects, "■", "Projects"),
    (ActiveTab::Jobs, "⚒", "Jobs"),
    (ActiveTab::Missions, "▶", "Missions"),
    (ActiveTab::Logs, "▸", "Logs"),
    (ActiveTab::Settings, "⚙", "Settings"),
];

fn badge_count(tab: ActiveTab, state: &AppState) -> usize {
    match tab {
        ActiveTab::Conversation => state.messages.len(),
        ActiveTab::Workspace => state.memory_count,
        ActiveTab::Projects => state.projects_overview.projects.len(),
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

pub(crate) fn nav_hit_areas(area: Rect) -> Vec<(ActiveTab, Rect)> {
    if area.height < 12 || area.width < 12 {
        return Vec::new();
    }

    NAV_ITEMS
        .iter()
        .enumerate()
        .map(|(index, (tab, _, _))| {
            let y = area.y.saturating_add(2 + index as u16);
            (
                *tab,
                Rect::new(area.x.saturating_add(1), y, area.width.saturating_sub(2), 1),
            )
        })
        .collect()
}

pub struct NavRailWidget {
    theme: Theme,
}

impl NavRailWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for NavRailWidget {
    fn id(&self) -> &str {
        "nav_rail"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::NavRail
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height < 8 || area.width < 12 {
            return;
        }

        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(self.theme.border_style())
            .style(self.theme.nav_style())
            .title(" Control ");
        let inner = block.inner(area);
        block.render(area, buf);

        let mut lines = vec![
            Line::from(Span::styled(" surfaces", self.theme.dim_style())),
            Line::from(""),
        ];

        for (tab, icon, label) in NAV_ITEMS {
            let is_active = state.active_tab == tab;
            let mut spans = vec![Span::styled(
                format!(" {icon} "),
                if is_active {
                    self.theme.selected_style()
                } else {
                    self.theme.nav_style()
                },
            )];
            spans.push(Span::styled(
                label,
                if is_active {
                    self.theme.selected_style()
                } else {
                    self.theme.dim_style().add_modifier(Modifier::BOLD)
                },
            ));
            let badge = badge_count(tab, state);
            if badge > 0 {
                spans.push(Span::styled(" ", self.theme.nav_style()));
                spans.push(Span::styled(
                    if badge > 99 {
                        "99+".to_string()
                    } else {
                        badge.to_string()
                    },
                    if is_active {
                        self.theme.selected_style()
                    } else {
                        Style::default().fg(self.theme.accent.to_color())
                    },
                ));
            }
            lines.push(Line::from(spans));
        }

        let footer_padding = inner.height.saturating_sub(lines.len() as u16 + 3) as usize;
        lines.extend(std::iter::repeat_n(Line::from(""), footer_padding));
        lines.push(Line::from(Span::styled(
            " ctrl-b projects",
            self.theme.dim_style(),
        )));
        lines.push(Line::from(Span::styled(
            " ctrl-l logs",
            self.theme.dim_style(),
        )));
        lines.push(Line::from(Span::styled(
            " /resume threads",
            self.theme.dim_style(),
        )));

        Paragraph::new(lines)
            .style(self.theme.nav_style())
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_rail_exposes_surface_order() {
        let items: Vec<_> = NAV_ITEMS.iter().map(|(tab, _, _)| *tab).collect();
        assert_eq!(
            items,
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
    fn nav_hit_areas_cover_settings_row() {
        let areas = nav_hit_areas(Rect::new(0, 0, 18, 20));
        assert!(
            areas
                .iter()
                .any(|(tab, rect)| { *tab == ActiveTab::Settings && rect.y == 8 })
        );
    }
}
