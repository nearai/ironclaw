//! Per-surface header block shown above the active content pane.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

pub struct SurfaceHeaderWidget {
    theme: Theme,
}

impl SurfaceHeaderWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    fn subtitle(&self, state: &AppState) -> String {
        match state.active_tab {
            ActiveTab::Conversation => format!(
                "{} messages  ·  {} active tools",
                state.messages.len(),
                state.active_tools.len()
            ),
            ActiveTab::Workspace => format!("{} memory entries loaded", state.memory_count),
            ActiveTab::Projects => format!(
                "{} projects  ·  {} attention items",
                state.projects_overview.projects.len(),
                state.projects_overview.attention.len()
            ),
            ActiveTab::Jobs => format!("{} tracked jobs", state.jobs.len()),
            ActiveTab::Missions => {
                let mission_count: usize = state
                    .projects_overview
                    .projects
                    .iter()
                    .map(|project| project.missions.len())
                    .sum();
                format!("{} missions across active projects", mission_count)
            }
            ActiveTab::Logs => format!("{} captured log lines", state.log_entries.len()),
            ActiveTab::Settings => format!("{} settings visible", state.settings.entries.len()),
        }
    }
}

impl TuiWidget for SurfaceHeaderWidget {
    fn id(&self) -> &str {
        "surface_header"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::SurfaceHeader
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height < 3 || area.width < 20 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.chrome_border_style())
            .style(self.theme.surface_header_style());
        let inner = block.inner(area);
        block.render(area, buf);

        let lines = vec![
            Line::from(vec![
                Span::styled(" control room", self.theme.dim_style()),
                Span::styled("  /  ", self.theme.dim_style()),
                Span::styled(state.active_tab.title(), self.theme.bold_accent_style()),
            ]),
            Line::from(vec![
                Span::styled(self.subtitle(state), self.theme.dim_style()),
                Span::styled("  ·  ", self.theme.dim_style()),
                Span::styled(state.status_text.clone(), self.theme.accent_style()),
            ]),
        ];

        Paragraph::new(lines)
            .style(self.theme.surface_header_style())
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_header_renders_active_title() {
        let widget = SurfaceHeaderWidget::new(Theme::dark());
        let state = AppState {
            active_tab: ActiveTab::Projects,
            status_text: "Ready".to_string(),
            ..Default::default()
        };
        let area = Rect::new(0, 0, 60, 4);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let mut text = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(text.contains("Projects"));
        assert!(text.contains("Ready"));
    }
}
