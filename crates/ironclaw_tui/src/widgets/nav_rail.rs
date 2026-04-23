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

/// Row offset inside the nav-rail area where the first tab row is drawn.
///
/// Must stay in sync with the [`NavRailWidget::render`] line list:
///   0: title (" IronClaw ") consumes the top border row
///   1: " control room"
///   2: " surfaces"
///   3: blank spacer
///   4: first tab (Chat)
const NAV_FIRST_ROW_OFFSET: u16 = 4;

pub(crate) fn nav_hit_areas(area: Rect) -> Vec<(ActiveTab, Rect)> {
    if area.height < 12 || area.width < 12 {
        return Vec::new();
    }

    NAV_ITEMS
        .iter()
        .enumerate()
        .map(|(index, (tab, _, _))| {
            let y = area.y.saturating_add(NAV_FIRST_ROW_OFFSET + index as u16);
            (*tab, Rect::new(area.x, y, area.width.saturating_sub(1), 1))
        })
        .collect()
}

pub struct NavRailWidget {
    theme: Theme,
    show_badges: bool,
    show_hints: bool,
}

impl NavRailWidget {
    pub fn new(theme: Theme, show_badges: bool, show_hints: bool) -> Self {
        Self {
            theme,
            show_badges,
            show_hints,
        }
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
            .border_style(self.theme.chrome_border_style())
            .style(self.theme.nav_style())
            .title(" IronClaw ");
        let inner = block.inner(area);
        block.render(area, buf);

        let mut lines = vec![
            Line::from(Span::styled(" control room", self.theme.bold_style())),
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
            if self.show_badges && badge > 0 {
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
                        Style::default().fg(self.theme.nav_active_fg.to_color())
                    },
                ));
            }
            lines.push(Line::from(spans));
        }

        let hint_rows = if self.show_hints { 4 } else { 0 };
        let footer_padding = inner.height.saturating_sub(lines.len() as u16 + hint_rows) as usize;
        lines.extend(std::iter::repeat_n(Line::from(""), footer_padding));
        if self.show_hints {
            lines.push(Line::from(Span::styled(
                " quick keys",
                self.theme.dim_style(),
            )));
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
        }

        Paragraph::new(lines)
            .style(self.theme.nav_style())
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_buffer(area: Rect, state: &AppState) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let widget = NavRailWidget::new(Theme::dark(), true, true);
        terminal
            .draw(|f| widget.render(area, f.buffer_mut(), state))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        (0..width).map(|x| buf[(x, y)].symbol()).collect()
    }

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

    /// Regression for a vertical drift between `nav_hit_areas` and the
    /// actual rendered rows: clicking a nav label must hit the tab whose
    /// text is drawn on that row.
    #[test]
    fn nav_hit_areas_align_with_rendered_rows() {
        let area = Rect::new(0, 0, 22, 24);
        let state = AppState::default();
        let buf = render_to_buffer(area, &state);
        let areas = nav_hit_areas(area);
        for (tab, rect) in areas {
            let label = NAV_ITEMS
                .iter()
                .find_map(|(t, _, l)| (*t == tab).then_some(*l))
                .expect("known tab");
            let row = row_text(&buf, rect.y, area.width);
            assert!(
                row.contains(label),
                "hit row y={} for {:?} should contain label {:?}, got {:?}",
                rect.y,
                tab,
                label,
                row,
            );
        }
    }
}
