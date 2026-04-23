//! Missions surface: list/detail shell built from the projects overview data.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{
    AppState, MissionsView, ProjectActivitySummary, ProjectMissionSummary, ProjectsOverviewData,
    TuiWidget,
};

pub enum MissionsMouseAction {
    OpenMission(String),
    OpenProject(String),
    OpenThreadDetail(String),
    BackToList,
}

pub struct MissionsWidget {
    theme: Theme,
}

pub struct MissionStateLookup<'a> {
    pub project_id: &'a str,
    pub project_name: &'a str,
    pub mission: &'a ProjectMissionSummary,
    pub recent_activity: &'a [ProjectActivitySummary],
}

impl MissionsWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    fn flattened_missions<'a>(
        &self,
        data: &'a ProjectsOverviewData,
    ) -> Vec<MissionStateLookup<'a>> {
        data.projects
            .iter()
            .flat_map(|project| {
                project.missions.iter().map(|mission| MissionStateLookup {
                    project_id: &project.id,
                    project_name: &project.name,
                    mission,
                    recent_activity: &project.recent_activity,
                })
            })
            .collect()
    }

    fn selected_mission<'a>(&self, state: &'a AppState) -> Option<MissionStateLookup<'a>> {
        self.flattened_missions(&state.projects_overview)
            .into_iter()
            .find(|entry| {
                Some(entry.mission.id.as_str())
                    == state.missions_surface.selected_mission_id.as_deref()
            })
    }

    pub fn action_at(
        &self,
        area: Rect,
        state: &AppState,
        column: u16,
        row: u16,
    ) -> Option<MissionsMouseAction> {
        if !rect_contains(area, column, row) {
            return None;
        }
        match state.missions_surface.view {
            MissionsView::List => {
                let first_row = area.y.saturating_add(4);
                let index = row.checked_sub(first_row)? as usize;
                // Each mission occupies 2 rows; map row to mission index.
                let mission_index = index / 2;
                self.flattened_missions(&state.projects_overview)
                    .get(mission_index)
                    .map(|entry| MissionsMouseAction::OpenMission(entry.mission.id.clone()))
            }
            MissionsView::Detail => {
                if row == area.y.saturating_add(1) && column <= area.x.saturating_add(12) {
                    return Some(MissionsMouseAction::BackToList);
                }

                let entry = self.selected_mission(state)?;
                if row == area.y.saturating_add(3) {
                    return Some(MissionsMouseAction::OpenProject(
                        entry.project_id.to_string(),
                    ));
                }

                let activity_start = area.y.saturating_add(10);
                let activity_index = row.checked_sub(activity_start)? as usize;
                entry
                    .recent_activity
                    .get(activity_index)
                    .map(|thread| MissionsMouseAction::OpenThreadDetail(thread.id.clone()))
            }
        }
    }

    fn status_icon(status: &str) -> &'static str {
        match status.to_lowercase().as_str() {
            "active" | "running" => "●",
            "paused" | "idle" => "○",
            "done" | "completed" | "finished" => "✓",
            "failed" | "error" => "✗",
            _ => "·",
        }
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Missions ");
        let inner = block.inner(area);
        block.render(area, buf);

        let missions = self.flattened_missions(&state.projects_overview);
        let active = missions
            .iter()
            .filter(|entry| entry.mission.status.eq_ignore_ascii_case("active"))
            .count();

        let mut lines = vec![
            Line::from(vec![
                Span::styled("◈ ", self.theme.accent_style()),
                Span::styled("Missions", self.theme.bold_accent_style()),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("{}", missions.len()),
                    self.theme.bold_accent_style(),
                ),
                Span::styled(" total  ·  ", self.theme.dim_style()),
                Span::styled(format!("{active}"), self.theme.warning_style()),
                Span::styled(" active", self.theme.dim_style()),
            ]),
            Line::from(Span::styled(
                "  ↑/↓ navigate   Enter open",
                self.theme.dim_style(),
            )),
        ];

        if missions.is_empty() {
            lines.push(Line::from(Span::styled(
                "No missions are available yet.",
                self.theme.dim_style(),
            )));
        } else {
            for (index, entry) in missions.iter().enumerate() {
                let is_selected = index == state.missions_surface.selected_index;
                let name_style = if is_selected {
                    self.theme.accent_style().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.fg.to_color())
                };
                let marker_style = if is_selected {
                    self.theme.accent_style().add_modifier(Modifier::BOLD)
                } else {
                    self.theme.dim_style()
                };
                let icon = Self::status_icon(&entry.mission.status);
                lines.push(Line::from(vec![
                    Span::styled(if is_selected { "▶ " } else { "  " }, marker_style),
                    Span::styled(icon, self.theme.accent_style()),
                    Span::styled(" ", Style::default()),
                    Span::styled(entry.mission.name.clone(), name_style),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(entry.project_name.to_string(), self.theme.dim_style()),
                ]));
                lines.push(Line::from(Span::styled(
                    format!(
                        "   {} · {} · {} threads",
                        entry.mission.status, entry.mission.cadence, entry.mission.thread_count
                    ),
                    self.theme.dim_style(),
                )));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn render_detail(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Mission detail ");
        let inner = block.inner(area);
        block.render(area, buf);

        let Some(entry) = self.selected_mission(state) else {
            Paragraph::new(Line::from(Span::styled(
                "Mission selection unavailable.",
                self.theme.dim_style(),
            )))
            .render(inner, buf);
            return;
        };

        // Line 0: back nav
        let mut lines = vec![Line::from(vec![
            Span::styled(
                "← Back",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  Esc", self.theme.dim_style()),
        ])];

        // Line 1: breadcrumb
        lines.push(Line::from(vec![
            Span::styled("Missions", self.theme.dim_style()),
            Span::styled(" › ", self.theme.dim_style()),
            Span::styled(entry.mission.name.clone(), self.theme.bold_style()),
        ]));

        // Line 2: blank
        lines.push(Line::from(""));

        // Line 3: project
        lines.push(Line::from(vec![
            Span::styled("Project  ", self.theme.dim_style()),
            Span::styled(entry.project_name.to_string(), self.theme.accent_style()),
            Span::styled("  · open project", self.theme.dim_style()),
        ]));

        let status_icon = Self::status_icon(&entry.mission.status);
        lines.push(Line::from(vec![
            Span::styled("Status   ", self.theme.dim_style()),
            Span::styled(
                format!("{} {}", status_icon, entry.mission.status),
                Style::default().fg(self.theme.fg.to_color()),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Cadence  ", self.theme.dim_style()),
            Span::styled(
                entry.mission.cadence.clone(),
                Style::default().fg(self.theme.fg.to_color()),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Threads  ", self.theme.dim_style()),
            Span::styled(
                format!("{}", entry.mission.thread_count),
                Style::default().fg(self.theme.fg.to_color()),
            ),
        ]));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Recent activity", self.theme.bold_accent_style()),
            Span::styled("  ↑/↓ navigate   Enter open", self.theme.dim_style()),
        ]));

        if entry.recent_activity.is_empty() {
            lines.push(Line::from(Span::styled("  none", self.theme.dim_style())));
        } else {
            for (idx, thread) in entry.recent_activity.iter().enumerate() {
                let is_selected = idx == state.missions_surface.activity_index;
                let row_style = if is_selected {
                    self.theme.accent_style().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.fg.to_color())
                };
                let bullet = if is_selected { "▶" } else { "•" };
                let mut spans = vec![
                    Span::styled(format!("  {bullet} "), self.theme.dim_style()),
                    Span::styled(thread.label.clone(), row_style),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(Self::status_icon(&thread.status), self.theme.accent_style()),
                    Span::styled(format!(" {}", thread.status), self.theme.dim_style()),
                ];
                if let Some(updated_at) = &thread.updated_at {
                    spans.push(Span::styled("  ·  ", self.theme.dim_style()));
                    spans.push(Span::styled(updated_at.clone(), self.theme.dim_style()));
                }
                if is_selected {
                    spans.push(Span::styled("  · open", self.theme.accent_style()));
                }
                lines.push(Line::from(spans));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }
}

impl TuiWidget for MissionsWidget {
    fn id(&self) -> &str {
        "missions"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height < 6 || area.width < 30 {
            return;
        }

        match state.missions_surface.view {
            MissionsView::List => self.render_list(area, buf, state),
            MissionsView::Detail => self.render_detail(area, buf, state),
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
        AppState, MissionsState, ProjectActivitySummary, ProjectMissionSummary,
        ProjectOverviewCard, ProjectsOverviewData,
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
                attention: Vec::new(),
                projects: vec![ProjectOverviewCard {
                    id: "project-1".to_string(),
                    name: "Alpha".to_string(),
                    description: String::new(),
                    health: "green".to_string(),
                    active_missions: 1,
                    threads_today: 2,
                    cost_today_usd: "$0.20".to_string(),
                    last_activity: None,
                    goals: Vec::new(),
                    missions: vec![ProjectMissionSummary {
                        id: "mission-1".to_string(),
                        name: "Theme migration".to_string(),
                        status: "Active".to_string(),
                        cadence: "manual".to_string(),
                        thread_count: 2,
                    }],
                    recent_activity: vec![ProjectActivitySummary {
                        id: "thread-1".to_string(),
                        label: "Refine tabs".to_string(),
                        status: "Running".to_string(),
                        updated_at: Some("now".to_string()),
                    }],
                }],
            },
            missions_surface: MissionsState {
                selected_index: 0,
                selected_mission_id: Some("mission-1".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn missions_list_renders_project_backed_rows() {
        let widget = MissionsWidget::new(Theme::dark());
        let state = sample_state();
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Missions"));
        assert!(text.contains("Theme migration"));
        assert!(text.contains("Alpha"));
    }

    #[test]
    fn missions_list_shows_keyboard_hint() {
        let widget = MissionsWidget::new(Theme::dark());
        let state = sample_state();
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Enter"));
    }

    #[test]
    fn mission_detail_renders_breadcrumb_and_activity() {
        let widget = MissionsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.missions_surface.open_mission("mission-1".to_string());
        let area = Rect::new(0, 0, 100, 25);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Back"), "should show Back nav");
        assert!(text.contains("Missions"), "should show breadcrumb root");
        assert!(text.contains("Theme migration"), "should show mission name");
        assert!(text.contains("Refine tabs"), "should show activity");
        assert!(text.contains("open project"), "should show project link");
    }

    #[test]
    fn mission_detail_highlights_selected_activity() {
        let widget = MissionsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.missions_surface.open_mission("mission-1".to_string());
        state.missions_surface.activity_index = 0;
        let area = Rect::new(0, 0, 100, 25);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        // Selected activity row should show the open prompt
        assert!(
            text.contains("open"),
            "selected activity should show open cue"
        );
    }

    #[test]
    fn action_at_maps_project_line_to_project_drilldown() {
        let widget = MissionsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.missions_surface.open_mission("mission-1".to_string());
        let area = Rect::new(0, 0, 100, 25);

        let action = widget.action_at(area, &state, area.x + 4, area.y + 3);

        assert!(matches!(
            action,
            Some(MissionsMouseAction::OpenProject(project_id)) if project_id == "project-1"
        ));
    }

    #[test]
    fn action_at_maps_recent_activity_to_thread_detail() {
        let widget = MissionsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.missions_surface.open_mission("mission-1".to_string());
        let area = Rect::new(0, 0, 100, 25);

        let action = widget.action_at(area, &state, area.x + 4, area.y + 10);

        assert!(matches!(
            action,
            Some(MissionsMouseAction::OpenThreadDetail(thread_id)) if thread_id == "thread-1"
        ));
    }

    #[test]
    fn selected_activity_thread_id_returns_correct_id() {
        let state = sample_state();
        let thread_id = state
            .missions_surface
            .selected_activity_thread_id(&state.projects_overview);
        // activity_index is 0, which maps to "thread-1"
        assert_eq!(thread_id.as_deref(), Some("thread-1"));
    }
}
