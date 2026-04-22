//! Projects surface: terminal-native control room mirroring the web overview
//! and drill-in flow.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::{truncate, wrap_text};
use crate::theme::Theme;

use super::{AppState, ProjectOverviewCard, ProjectsView, TuiWidget};

pub enum ProjectsMouseAction {
    OpenProject(String),
    OpenMission(String),
    OpenThreadDetail(String),
    BackToOverview,
}

pub struct ProjectsWidget {
    theme: Theme,
}

impl ProjectsWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub fn action_at(
        &self,
        area: Rect,
        state: &AppState,
        column: u16,
        row: u16,
    ) -> Option<ProjectsMouseAction> {
        if !rect_contains(area, column, row) {
            return None;
        }

        match state.projects.view {
            ProjectsView::Overview => {
                let first_row = area.y.saturating_add(4);
                let index = row.checked_sub(first_row)? as usize;
                state
                    .projects_overview
                    .projects
                    .get(index)
                    .map(|project| ProjectsMouseAction::OpenProject(project.id.clone()))
            }
            ProjectsView::ProjectDetail
            | ProjectsView::MissionDetail
            | ProjectsView::ThreadDetail => {
                if row == area.y.saturating_add(1) && column <= area.x.saturating_add(14) {
                    return Some(ProjectsMouseAction::BackToOverview);
                }

                let project = self.current_project(state)?;
                let line_index = row.checked_sub(area.y.saturating_add(1))? as usize;
                let mut current_line = 0usize;

                current_line += 1; // back/header row
                if !project.description.is_empty() {
                    current_line += 1;
                }
                current_line += 1; // blank
                current_line += 1; // goals heading
                if project.goals.is_empty() {
                    current_line += 1;
                } else {
                    current_line += project
                        .goals
                        .iter()
                        .map(|goal| {
                            wrap_text(
                                &format!("• {goal}"),
                                area.width.saturating_sub(4) as usize,
                                Style::default().fg(self.theme.fg.to_color()),
                            )
                            .len()
                        })
                        .sum::<usize>();
                }
                current_line += 1; // blank
                current_line += 1; // missions heading

                let missions_start = current_line;
                let missions_end = missions_start + project.missions.len();
                if (missions_start..missions_end).contains(&line_index) {
                    return project
                        .missions
                        .get(line_index - missions_start)
                        .map(|mission| ProjectsMouseAction::OpenMission(mission.id.clone()));
                }

                current_line = missions_end;
                current_line += 1; // blank
                current_line += 1; // recent activity heading

                let activity_start = current_line;
                let activity_end = activity_start + project.recent_activity.len();
                if (activity_start..activity_end).contains(&line_index) {
                    return project
                        .recent_activity
                        .get(line_index - activity_start)
                        .map(|thread| ProjectsMouseAction::OpenThreadDetail(thread.id.clone()));
                }

                None
            }
        }
    }

    fn current_project<'a>(&self, state: &'a AppState) -> Option<&'a ProjectOverviewCard> {
        let selected_id = state.projects.selected_project_id.as_deref()?;
        state
            .projects_overview
            .projects
            .iter()
            .find(|project| project.id == selected_id)
    }

    fn render_overview(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Projects ");
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
            Span::styled("◈ ", self.theme.accent_style()),
            Span::styled("Projects control room", self.theme.bold_accent_style()),
        ])];

        if state.projects_overview.attention.is_empty() {
            lines.push(Line::from(Span::styled(
                "Needs attention: none",
                self.theme.dim_style(),
            )));
        } else {
            let count = state.projects_overview.attention.len();
            lines.push(Line::from(vec![
                Span::styled("Needs attention ", self.theme.warning_style()),
                Span::styled(format!("{count}"), self.theme.bold_accent_style()),
            ]));
        }

        lines.push(Line::from(""));

        if state.projects_overview.projects.is_empty() {
            lines.push(Line::from(Span::styled(
                "No projects available yet.",
                self.theme.dim_style(),
            )));
        } else {
            for (index, project) in state.projects_overview.projects.iter().enumerate() {
                let is_selected = index == state.projects.selected_overview;
                let prefix = if is_selected { "▶" } else { " " };
                let health = if project.health.eq_ignore_ascii_case("green") {
                    self.theme.success_style()
                } else if project.health.eq_ignore_ascii_case("yellow") {
                    self.theme.warning_style()
                } else {
                    self.theme.error_style()
                };
                let title_style = if is_selected {
                    self.theme.accent_style().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.fg.to_color())
                };
                let subtitle = format!(
                    "{} active · {} threads today · {}",
                    project.active_missions, project.threads_today, project.cost_today_usd
                );
                lines.push(Line::from(vec![
                    Span::styled(format!("{prefix} "), title_style),
                    Span::styled("● ", health),
                    Span::styled(project.name.clone(), title_style),
                ]));
                lines.push(Line::from(Span::styled(
                    format!(
                        "   {}",
                        truncate(&subtitle, inner.width.saturating_sub(3) as usize)
                    ),
                    self.theme.dim_style(),
                )));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn render_project_detail(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Project detail ");
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let Some(project) = self.current_project(state) else {
            Paragraph::new(Line::from(Span::styled(
                "Project selection is unavailable.",
                self.theme.dim_style(),
            )))
            .render(inner, buf);
            return;
        };

        let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
            Span::styled(
                "← Back",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(project.name.clone(), self.theme.bold_style()),
        ])];

        if !project.description.is_empty() {
            lines.push(Line::from(Span::styled(
                project.description.clone(),
                self.theme.dim_style(),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Goals",
            self.theme.bold_accent_style(),
        )));
        if project.goals.is_empty() {
            lines.push(Line::from(Span::styled("  none", self.theme.dim_style())));
        } else {
            for goal in &project.goals {
                for wrapped in wrap_text(
                    &format!("• {goal}"),
                    inner.width.saturating_sub(2) as usize,
                    Style::default().fg(self.theme.fg.to_color()),
                ) {
                    lines.push(wrapped);
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Missions",
            self.theme.bold_accent_style(),
        )));
        if project.missions.is_empty() {
            lines.push(Line::from(Span::styled("  none", self.theme.dim_style())));
        } else {
            for mission in &project.missions {
                lines.push(Line::from(vec![
                    Span::styled("  • ", self.theme.dim_style()),
                    Span::styled(mission.name.clone(), self.theme.accent_style()),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(mission.status.clone(), self.theme.warning_style()),
                    Span::styled("  · open mission", self.theme.dim_style()),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Recent activity",
            self.theme.bold_accent_style(),
        )));
        if project.recent_activity.is_empty() {
            lines.push(Line::from(Span::styled("  none", self.theme.dim_style())));
        } else {
            for thread in &project.recent_activity {
                let mut parts = vec![
                    Span::styled("  • ", self.theme.dim_style()),
                    Span::styled(thread.label.clone(), self.theme.bold_style()),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(thread.status.clone(), self.theme.accent_style()),
                ];
                if let Some(updated_at) = &thread.updated_at {
                    parts.push(Span::styled("  ·  ", self.theme.dim_style()));
                    parts.push(Span::styled(updated_at.clone(), self.theme.dim_style()));
                }
                parts.push(Span::styled("  · open", self.theme.dim_style()));
                lines.push(Line::from(parts));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }
}

impl TuiWidget for ProjectsWidget {
    fn id(&self) -> &str {
        "projects"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height < 6 || area.width < 30 {
            return;
        }

        match state.projects.view {
            ProjectsView::Overview => self.render_overview(area, buf, state),
            ProjectsView::ProjectDetail
            | ProjectsView::MissionDetail
            | ProjectsView::ThreadDetail => self.render_project_detail(area, buf, state),
        }
    }
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{
        ProjectActivitySummary, ProjectAttentionItem, ProjectMissionSummary, ProjectOverviewCard,
        ProjectsOverviewData, ProjectsState,
    };

    fn buffer_text(buf: &Buffer, area: Rect) -> String {
        let mut lines = Vec::new();
        for y in area.top()..area.bottom() {
            let mut line = String::new();
            for x in area.left()..area.right() {
                line.push_str(buf[(x, y)].symbol());
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    fn sample_state() -> AppState {
        AppState {
            projects_overview: ProjectsOverviewData {
                attention: vec![ProjectAttentionItem {
                    kind: "gate".to_string(),
                    project_id: "p1".to_string(),
                    project_name: "Alpha".to_string(),
                    message: "Awaiting approval".to_string(),
                    thread_id: Some("t1".to_string()),
                }],
                projects: vec![ProjectOverviewCard {
                    id: "p1".to_string(),
                    name: "Alpha".to_string(),
                    description: "Primary autonomous workspace".to_string(),
                    health: "green".to_string(),
                    active_missions: 2,
                    threads_today: 4,
                    cost_today_usd: "$1.25".to_string(),
                    last_activity: Some("5m ago".to_string()),
                    goals: vec!["Ship the new TUI".to_string()],
                    missions: vec![ProjectMissionSummary {
                        id: "m1".to_string(),
                        name: "Theme migration".to_string(),
                        status: "Active".to_string(),
                        cadence: "manual".to_string(),
                        thread_count: 2,
                    }],
                    recent_activity: vec![ProjectActivitySummary {
                        id: "t1".to_string(),
                        label: "Refine control room".to_string(),
                        status: "Running".to_string(),
                        updated_at: Some("just now".to_string()),
                    }],
                }],
            },
            projects: ProjectsState {
                selected_overview: 0,
                selected_project_id: Some("p1".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn projects_overview_renders_attention_and_cards() {
        let widget = ProjectsWidget::new(Theme::dark());
        let state = sample_state();
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Projects control room"));
        assert!(text.contains("Needs attention"));
        assert!(text.contains("Alpha"));
    }

    #[test]
    fn projects_detail_renders_goals_and_activity() {
        let widget = ProjectsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.projects.open_project("p1".to_string());
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Back"));
        assert!(text.contains("Goals"));
        assert!(text.contains("Theme migration"));
        assert!(text.contains("open mission"));
        assert!(text.contains("Refine control room"));
        assert!(text.contains("open"));
    }

    #[test]
    fn action_at_maps_overview_row_to_project() {
        let widget = ProjectsWidget::new(Theme::dark());
        let state = sample_state();
        let area = Rect::new(0, 0, 100, 20);

        let action = widget.action_at(area, &state, area.x + 2, area.y + 4);

        match action {
            Some(ProjectsMouseAction::OpenProject(id)) => assert_eq!(id, "p1"),
            _ => panic!("expected project open action"),
        }
    }

    #[test]
    fn action_at_maps_project_detail_mission_to_missions_surface() {
        let widget = ProjectsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.projects.open_project("p1".to_string());
        let area = Rect::new(0, 0, 100, 20);

        let action = widget.action_at(area, &state, area.x + 4, area.y + 8);

        assert!(matches!(
            action,
            Some(ProjectsMouseAction::OpenMission(mission_id)) if mission_id == "m1"
        ));
    }

    #[test]
    fn action_at_maps_project_detail_activity_to_thread_detail() {
        let widget = ProjectsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.projects.open_project("p1".to_string());
        let area = Rect::new(0, 0, 100, 20);

        let action = widget.action_at(area, &state, area.x + 4, area.y + 11);

        assert!(matches!(
            action,
            Some(ProjectsMouseAction::OpenThreadDetail(thread_id)) if thread_id == "t1"
        ));
    }
}
