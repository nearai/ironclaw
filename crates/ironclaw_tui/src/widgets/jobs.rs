//! Jobs surface: list/detail shell for sandbox jobs.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::format_duration;
use crate::theme::Theme;

use super::{AppState, JobDetailTab, JobInfo, JobsView, TuiWidget};

pub enum JobsMouseAction {
    OpenJob(String),
    BackToList,
    SelectTab(JobDetailTab),
}

pub struct JobsWidget {
    theme: Theme,
}

impl JobsWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub fn action_at(
        &self,
        area: Rect,
        state: &AppState,
        column: u16,
        row: u16,
    ) -> Option<JobsMouseAction> {
        if !rect_contains(area, column, row) {
            return None;
        }

        match state.jobs_surface.view {
            JobsView::List => {
                let first_row = area.y.saturating_add(4);
                let index = row.checked_sub(first_row)? as usize;
                state
                    .jobs
                    .get(index)
                    .map(|job| JobsMouseAction::OpenJob(job.id.clone()))
            }
            JobsView::Detail => {
                if row == area.y.saturating_add(1) && column <= area.x.saturating_add(12) {
                    return Some(JobsMouseAction::BackToList);
                }
                if row == area.y.saturating_add(3) {
                    let overview_range = area.x.saturating_add(2)..area.x.saturating_add(12);
                    let activity_range = area.x.saturating_add(13)..area.x.saturating_add(23);
                    let files_range = area.x.saturating_add(24)..area.x.saturating_add(31);
                    if overview_range.contains(&column) {
                        return Some(JobsMouseAction::SelectTab(JobDetailTab::Overview));
                    }
                    if activity_range.contains(&column) {
                        return Some(JobsMouseAction::SelectTab(JobDetailTab::Activity));
                    }
                    if files_range.contains(&column) {
                        return Some(JobsMouseAction::SelectTab(JobDetailTab::Files));
                    }
                }
                None
            }
        }
    }

    fn selected_job<'a>(&self, state: &'a AppState) -> Option<&'a JobInfo> {
        let selected_id = state.jobs_surface.selected_job_id.as_deref()?;
        state.jobs.iter().find(|job| job.id == selected_id)
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Jobs ");
        let inner = block.inner(area);
        block.render(area, buf);

        let total = state.jobs.len();
        let running = state
            .jobs
            .iter()
            .filter(|job| matches!(job.status, super::JobStatus::Running))
            .count();
        let failed = state
            .jobs
            .iter()
            .filter(|job| matches!(job.status, super::JobStatus::Failed))
            .count();

        let mut lines = vec![
            Line::from(vec![
                Span::styled("◈ ", self.theme.accent_style()),
                Span::styled("Jobs", self.theme.bold_accent_style()),
            ]),
            Line::from(vec![
                Span::styled(format!("{total}"), self.theme.bold_accent_style()),
                Span::styled(" total  ·  ", self.theme.dim_style()),
                Span::styled(format!("{running}"), self.theme.warning_style()),
                Span::styled(" running  ·  ", self.theme.dim_style()),
                Span::styled(format!("{failed}"), self.theme.error_style()),
                Span::styled(" failed", self.theme.dim_style()),
            ]),
            Line::from(""),
        ];

        if state.jobs.is_empty() {
            lines.push(Line::from(Span::styled(
                "No jobs recorded yet.",
                self.theme.dim_style(),
            )));
        } else {
            for (index, job) in state.jobs.iter().enumerate() {
                let is_selected = index == state.jobs_surface.selected_index;
                let marker_style = if is_selected {
                    self.theme.accent_style().add_modifier(Modifier::BOLD)
                } else {
                    self.theme.dim_style()
                };
                let status_style = match job.status {
                    super::JobStatus::Running => self.theme.warning_style(),
                    super::JobStatus::Completed => self.theme.success_style(),
                    super::JobStatus::Failed => self.theme.error_style(),
                    super::JobStatus::Pending => self.theme.dim_style(),
                };
                lines.push(Line::from(vec![
                    Span::styled(if is_selected { "▶ " } else { "  " }, marker_style),
                    Span::styled(job.title.clone(), marker_style),
                    Span::styled("  ·  ", self.theme.dim_style()),
                    Span::styled(job.status.to_string(), status_style),
                ]));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn render_detail(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Job detail ");
        let inner = block.inner(area);
        block.render(area, buf);

        let Some(job) = self.selected_job(state) else {
            Paragraph::new(Line::from(Span::styled(
                "Job selection unavailable.",
                self.theme.dim_style(),
            )))
            .render(inner, buf);
            return;
        };

        let mut lines = vec![Line::from(vec![
            Span::styled(
                "← Back",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(job.title.clone(), self.theme.bold_style()),
        ])];
        lines.push(Line::from(Span::styled(
            format!(
                "Tabs: {} | {} | {}",
                tab_label(JobDetailTab::Overview, state),
                tab_label(JobDetailTab::Activity, state),
                tab_label(JobDetailTab::Files, state)
            ),
            self.theme.dim_style(),
        )));
        lines.push(Line::from(""));

        match state.jobs_surface.detail_tab {
            JobDetailTab::Overview => {
                let duration = chrono::Utc::now()
                    .signed_duration_since(job.started_at)
                    .num_seconds()
                    .unsigned_abs();
                lines.push(Line::from(format!("ID: {}", job.id)));
                lines.push(Line::from(format!("Status: {}", job.status)));
                lines.push(Line::from(format!(
                    "Started: {}",
                    job.started_at.format("%Y-%m-%d %H:%M UTC")
                )));
                lines.push(Line::from(format!(
                    "Duration: {}",
                    format_duration(duration)
                )));
            }
            JobDetailTab::Activity => {
                let mut matched = false;
                for message in state
                    .messages
                    .iter()
                    .filter(|message| message.content.contains("[job]"))
                {
                    matched = true;
                    lines.push(Line::from(Span::styled(
                        message.content.clone(),
                        Style::default().fg(self.theme.fg.to_color()),
                    )));
                }
                if !matched {
                    lines.push(Line::from(Span::styled(
                        "No job activity captured in this session yet.",
                        self.theme.dim_style(),
                    )));
                }
            }
            JobDetailTab::Files => {
                lines.push(Line::from(Span::styled(
                    "Job file browsing will reuse the Workspace tree in a follow-up slice.",
                    self.theme.dim_style(),
                )));
                lines.push(Line::from(Span::styled(
                    format!("Workspace: {}", state.workspace_path),
                    self.theme.dim_style(),
                )));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }
}

impl TuiWidget for JobsWidget {
    fn id(&self) -> &str {
        "jobs"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height < 6 || area.width < 30 {
            return;
        }

        match state.jobs_surface.view {
            JobsView::List => self.render_list(area, buf, state),
            JobsView::Detail => self.render_detail(area, buf, state),
        }
    }
}

fn tab_label(tab: JobDetailTab, state: &AppState) -> &'static str {
    if state.jobs_surface.detail_tab == tab {
        match tab {
            JobDetailTab::Overview => "[Overview]",
            JobDetailTab::Activity => "[Activity]",
            JobDetailTab::Files => "[Files]",
        }
    } else {
        match tab {
            JobDetailTab::Overview => "Overview",
            JobDetailTab::Activity => "Activity",
            JobDetailTab::Files => "Files",
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
    use crate::widgets::{AppState, ChatMessage, JobStatus, JobsState, MessageRole};

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
            jobs: vec![JobInfo {
                id: "job-1".to_string(),
                title: "Backfill".to_string(),
                status: JobStatus::Running,
                started_at: chrono::Utc::now(),
            }],
            jobs_surface: JobsState {
                selected_index: 0,
                selected_job_id: Some("job-1".to_string()),
                ..Default::default()
            },
            messages: vec![ChatMessage {
                role: MessageRole::System,
                content: "[job] Backfill (job-1)".to_string(),
                timestamp: chrono::Utc::now(),
                cost_summary: None,
            }],
            workspace_path: "/tmp/workspace".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn jobs_list_renders_job_rows() {
        let widget = JobsWidget::new(Theme::dark());
        let state = sample_state();
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Jobs"));
        assert!(text.contains("Backfill"));
    }

    #[test]
    fn jobs_detail_renders_activity_tab() {
        let widget = JobsWidget::new(Theme::dark());
        let mut state = sample_state();
        state.jobs_surface.open_job("job-1".to_string());
        state.jobs_surface.detail_tab = JobDetailTab::Activity;
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Back"));
        assert!(text.contains("[job] Backfill"));
    }
}
