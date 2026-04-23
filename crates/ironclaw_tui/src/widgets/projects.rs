//! Projects surface: terminal-native control room mirroring the web overview
//! and drill-in flow.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
                let inner_y = area.y.saturating_add(1);
                let line = row.checked_sub(inner_y)? as usize;
                let attention_start = 2usize;
                let attention_end = attention_start + state.projects_overview.attention.len();
                if (attention_start..attention_end).contains(&line) {
                    return state
                        .projects_overview
                        .attention
                        .get(line - attention_start)
                        .map(attention_action);
                }

                let projects_start = attention_end.saturating_add(1);
                let project_line = line.checked_sub(projects_start)?;
                let index = project_line / 2;
                if project_line % 2 <= 1 {
                    return state
                        .projects_overview
                        .projects
                        .get(index)
                        .map(|project| ProjectsMouseAction::OpenProject(project.id.clone()));
                }
                None
            }
            ProjectsView::ProjectDetail
            | ProjectsView::MissionDetail
            | ProjectsView::ThreadDetail => {
                let inner = inner_with_hints(area);
                if row == inner.y && column <= inner.x.saturating_add(14) {
                    return Some(ProjectsMouseAction::BackToOverview);
                }

                let project = self.current_project(state)?;
                let sections = detail_sections(inner, project);

                let mission_index =
                    row.checked_sub(sections.missions.y.saturating_add(1))? as usize;
                if row > sections.missions.y
                    && row < sections.missions.bottom().saturating_sub(1)
                    && mission_index < project.missions.len()
                {
                    return project
                        .missions
                        .get(mission_index)
                        .map(|mission| ProjectsMouseAction::OpenMission(mission.id.clone()));
                }

                let activity_index =
                    row.checked_sub(sections.activity.y.saturating_add(1))? as usize;
                if row > sections.activity.y
                    && row < sections.activity.bottom().saturating_sub(1)
                    && activity_index < project.recent_activity.len()
                {
                    return project
                        .recent_activity
                        .get(activity_index)
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

        if let Some(error) = state.projects.overview_error.as_deref() {
            Paragraph::new(Line::from(Span::styled(
                format!("Error: {error}"),
                self.theme.error_style(),
            )))
            .render(inner, buf);
            return;
        }

        if !state.projects.overview_loaded
            && state.projects_overview.attention.is_empty()
            && state.projects_overview.projects.is_empty()
        {
            let tick = state.spinner.frame(state.tick_count, 33);
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{tick} "), self.theme.accent_style()),
                Span::styled("Loading projects...", self.theme.dim_style()),
            ]))
            .render(inner, buf);
            return;
        }

        let [content, hints] = split_content_and_hints(inner);

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
            for (index, item) in state.projects_overview.attention.iter().enumerate() {
                let is_selected = index == state.projects.selected_overview;
                let style = if is_selected {
                    self.theme.accent_style().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.fg.to_color())
                };
                lines.push(Line::from(vec![
                    Span::styled(if is_selected { "▶ " } else { "  " }, style),
                    Span::styled(item.project_name.clone(), style),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(item.kind.clone(), self.theme.warning_style()),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(
                        truncate(&item.message, content.width.saturating_sub(8) as usize),
                        self.theme.dim_style(),
                    ),
                ]));
            }
        }

        lines.push(Line::from(""));

        if state.projects_overview.projects.is_empty() {
            lines.push(Line::from(Span::styled(
                "No projects yet",
                self.theme.dim_style(),
            )));
        } else {
            for (index, project) in state.projects_overview.projects.iter().enumerate() {
                let overview_index = state.projects_overview.attention.len() + index;
                let is_selected = overview_index == state.projects.selected_overview;
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
                        truncate(&subtitle, content.width.saturating_sub(3) as usize)
                    ),
                    self.theme.dim_style(),
                )));
            }
        }

        Paragraph::new(lines).render(content, buf);
        render_hints(hints, buf, &self.theme, state.projects.view);
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

        let [content, hints] = split_content_and_hints(inner);
        let sections = detail_sections(content, project);

        Paragraph::new(Line::from(vec![
            Span::styled(
                "← Back",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled("Projects", self.theme.dim_style()),
            Span::styled(" > ", self.theme.dim_style()),
            Span::styled(project.name.clone(), self.theme.bold_style()),
        ]))
        .render(sections.header, buf);

        let mut overview_lines: Vec<Line<'static>> = Vec::new();
        if !project.description.is_empty() {
            overview_lines.push(Line::from(Span::styled(
                project.description.clone(),
                self.theme.dim_style(),
            )));
        }
        overview_lines.push(Line::from(vec![
            Span::styled("Health: ", self.theme.dim_style()),
            Span::styled(project.health.clone(), self.theme.accent_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(
                format!("{} active missions", project.active_missions),
                self.theme.dim_style(),
            ),
        ]));
        if project.goals.is_empty() {
            overview_lines.push(Line::from(Span::styled(
                "No goals recorded",
                self.theme.dim_style(),
            )));
        } else {
            for goal in &project.goals {
                for wrapped in wrap_text(
                    &format!("• {goal}"),
                    sections.overview.width.saturating_sub(4) as usize,
                    Style::default().fg(self.theme.fg.to_color()),
                ) {
                    overview_lines.push(wrapped);
                }
            }
        }
        render_section(
            sections.overview,
            buf,
            " Overview ",
            overview_lines,
            &self.theme,
        );

        let mut mission_lines: Vec<Line<'static>> = Vec::new();
        if project.missions.is_empty() {
            mission_lines.push(Line::from(Span::styled(
                "No missions yet",
                self.theme.dim_style(),
            )));
        } else {
            for mission in &project.missions {
                mission_lines.push(Line::from(vec![
                    Span::styled("• ", self.theme.dim_style()),
                    Span::styled(mission.name.clone(), self.theme.accent_style()),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(mission.status.clone(), self.theme.warning_style()),
                    Span::styled("  · open mission", self.theme.dim_style()),
                ]));
            }
        }
        render_section(
            sections.missions,
            buf,
            " Missions ",
            mission_lines,
            &self.theme,
        );

        let mut activity_lines: Vec<Line<'static>> = Vec::new();
        if project.recent_activity.is_empty() {
            activity_lines.push(Line::from(Span::styled(
                "No recent activity",
                self.theme.dim_style(),
            )));
        } else {
            for thread in &project.recent_activity {
                let mut parts = vec![
                    Span::styled("• ", self.theme.dim_style()),
                    Span::styled(thread.label.clone(), self.theme.bold_style()),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(thread.status.clone(), self.theme.accent_style()),
                ];
                if let Some(updated_at) = &thread.updated_at {
                    parts.push(Span::styled("  ·  ", self.theme.dim_style()));
                    parts.push(Span::styled(updated_at.clone(), self.theme.dim_style()));
                }
                parts.push(Span::styled("  · open", self.theme.dim_style()));
                activity_lines.push(Line::from(parts));
            }
        }
        render_section(
            sections.activity,
            buf,
            " Recent Activity ",
            activity_lines,
            &self.theme,
        );
        render_hints(hints, buf, &self.theme, state.projects.view);
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

fn attention_action(item: &super::ProjectAttentionItem) -> ProjectsMouseAction {
    if let Some(thread_id) = &item.thread_id {
        ProjectsMouseAction::OpenThreadDetail(thread_id.clone())
    } else {
        ProjectsMouseAction::OpenProject(item.project_id.clone())
    }
}

fn inner_with_hints(area: Rect) -> Rect {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    split_content_and_hints(inner)[0]
}

fn split_content_and_hints(inner: Rect) -> [Rect; 2] {
    if inner.height <= 1 {
        return [inner, Rect::new(inner.x, inner.y, inner.width, 0)];
    }
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .areas(inner)
}

struct DetailSections {
    header: Rect,
    overview: Rect,
    missions: Rect,
    activity: Rect,
}

fn detail_sections(area: Rect, project: &ProjectOverviewCard) -> DetailSections {
    if area.height <= 1 {
        return DetailSections {
            header: area,
            overview: Rect::new(area.x, area.y, area.width, 0),
            missions: Rect::new(area.x, area.y, area.width, 0),
            activity: Rect::new(area.x, area.y, area.width, 0),
        };
    }

    let goal_rows = project.goals.len().max(1) as u16;
    let overview_height = (goal_rows + 4).clamp(4, 8);
    let mission_height = (project.missions.len() as u16 + 2).clamp(3, 7);
    let [header, overview, missions, activity] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(overview_height),
            Constraint::Length(mission_height),
            Constraint::Min(3),
        ])
        .areas(area);

    DetailSections {
        header,
        overview,
        missions,
        activity,
    }
}

fn render_section(
    area: Rect,
    buf: &mut Buffer,
    title: &'static str,
    lines: Vec<Line<'static>>,
    theme: &Theme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style())
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height > 0 && inner.width > 0 {
        Paragraph::new(lines).render(inner, buf);
    }
}

fn render_hints(area: Rect, buf: &mut Buffer, theme: &Theme, view: ProjectsView) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let text = match view {
        ProjectsView::Overview => "[Enter] Open  [n] New Mission",
        ProjectsView::ProjectDetail | ProjectsView::MissionDetail | ProjectsView::ThreadDetail => {
            "[Enter] Open  [n] New Mission  [Esc] Back"
        }
    };
    Paragraph::new(Line::from(Span::styled(text, theme.dim_style()))).render(area, buf);
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
        assert!(text.contains("Overview"));
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

        let action = widget.action_at(area, &state, area.x + 2, area.y + 5);

        match action {
            Some(ProjectsMouseAction::OpenProject(id)) => assert_eq!(id, "p1"),
            _ => panic!("expected project open action"),
        }
    }

    #[test]
    fn action_at_maps_attention_row_to_thread_detail() {
        let widget = ProjectsWidget::new(Theme::dark());
        let state = sample_state();
        let area = Rect::new(0, 0, 100, 20);

        let action = widget.action_at(area, &state, area.x + 2, area.y + 3);

        assert!(matches!(
            action,
            Some(ProjectsMouseAction::OpenThreadDetail(thread_id)) if thread_id == "t1"
        ));
    }

    #[test]
    fn projects_overview_renders_loading_empty_and_error_states() {
        let widget = ProjectsWidget::new(Theme::dark());
        let area = Rect::new(0, 0, 100, 10);

        let mut loading_buf = Buffer::empty(area);
        widget.render(area, &mut loading_buf, &AppState::default());
        assert!(buffer_text(&loading_buf, area).contains("Loading projects"));

        let empty_state = AppState {
            projects: ProjectsState {
                overview_loaded: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut empty_buf = Buffer::empty(area);
        widget.render(area, &mut empty_buf, &empty_state);
        assert!(buffer_text(&empty_buf, area).contains("No projects yet"));

        let error_state = AppState {
            projects: ProjectsState {
                overview_error: Some("backend unavailable".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut error_buf = Buffer::empty(area);
        widget.render(area, &mut error_buf, &error_state);
        assert!(buffer_text(&error_buf, area).contains("backend unavailable"));
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
