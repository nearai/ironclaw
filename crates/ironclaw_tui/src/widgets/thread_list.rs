//! Activity sidebar panel — jobs, routines, and threads.
//!
//! Renders a compact status table with three sections:
//!
//! ```text
//! JOBS (2) ─────────────
//! ● build-frontend  running  3m
//! ✓ daily-sync      done     15m
//!
//! ROUTINES (1) ─────────
//! ◆ issue-watch  github→issue  on
//!
//! THREADS (1) ──────────
//! ● main           active   2m
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

use crate::layout::TuiSlot;
use crate::render::truncate;
use crate::theme::Theme;

use super::{AppState, JobStatus, ThreadStatus, TuiWidget};

/// Status icon for each job state.
fn job_icon(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "\u{25CB}",   // ○
        JobStatus::Running => "\u{25CF}",   // ●
        JobStatus::Completed => "\u{2713}", // ✓
        JobStatus::Failed => "\u{2717}",    // ✗
    }
}

/// Status icon for each thread state.
fn thread_icon(status: ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::Active => "\u{25CF}",    // ●
        ThreadStatus::Idle => "\u{25CB}",      // ○
        ThreadStatus::Completed => "\u{2713}", // ✓
        ThreadStatus::Failed => "\u{2717}",    // ✗
    }
}

/// Format a duration in seconds into a compact human-readable string.
fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h")
        }
    }
}

pub struct ThreadListWidget {
    theme: Theme,
}

impl ThreadListWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Pick the style for a job status icon and text.
    fn job_status_style(&self, status: JobStatus) -> ratatui::style::Style {
        match status {
            JobStatus::Pending => self.theme.dim_style(),
            JobStatus::Running => self.theme.accent_style(),
            JobStatus::Completed => self.theme.success_style(),
            JobStatus::Failed => self.theme.error_style(),
        }
    }

    /// Pick the style for a thread's status icon and text.
    fn thread_status_style(&self, status: ThreadStatus) -> ratatui::style::Style {
        match status {
            ThreadStatus::Active => self.theme.accent_style(),
            ThreadStatus::Idle => self.theme.dim_style(),
            ThreadStatus::Completed => self.theme.success_style(),
            ThreadStatus::Failed => self.theme.error_style(),
        }
    }

    /// Render a section header: " LABEL (count) ────"
    fn render_section_header<'a>(&self, label: &str, count: usize, width: usize) -> Line<'a> {
        let header_text = format!(" {label} ({count})");
        let rule_len = width.saturating_sub(header_text.len() + 1);
        let rule = if rule_len > 0 {
            format!(" {}", "\u{2500}".repeat(rule_len))
        } else {
            String::new()
        };
        Line::from(vec![
            Span::styled(header_text, self.theme.bold_style()),
            Span::styled(rule, self.theme.dim_style()),
        ])
    }
}

impl TuiWidget for ThreadListWidget {
    fn id(&self) -> &str {
        "thread_list"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::SidebarSection
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 6 {
            return;
        }

        // Bordered block with "Activity" title
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(Span::styled(
                " Activity ",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width < 4 {
            return;
        }

        let width = inner.width as usize;
        let mut lines: Vec<Line<'_>> = Vec::new();

        // Column layout: " {icon} {name}  {status}  {uptime}"
        let fixed_cols: usize = 3 + 6 + 8 + 4; // icon + status + uptime + spacing
        let max_name_len = width.saturating_sub(fixed_cols).max(4);

        let now = chrono::Utc::now();

        // Global empty state hint
        let has_any_data = !state.jobs.is_empty()
            || !state.routines.is_empty()
            || !state.threads.is_empty()
            || state.sandbox_status.is_some()
            || state.secrets_status.is_some();
        if !has_any_data {
            lines.push(Line::from(Span::styled(
                " No active jobs or routines",
                self.theme.dim_style(),
            )));
            let paragraph = ratatui::widgets::Paragraph::new(lines);
            paragraph.render(inner, buf);
            return;
        }

        // ── SYSTEM section ────────────────────────────────────────────
        let system_items =
            state.sandbox_status.is_some() as usize + state.secrets_status.is_some() as usize;
        if system_items > 0 {
            lines.push(self.render_section_header("SYSTEM", system_items, width));

            if let Some(ref sandbox) = state.sandbox_status {
                let (icon, style) = if sandbox.docker_available {
                    ("\u{25CF}", self.theme.success_style()) // ●
                } else {
                    ("\u{25CB}", self.theme.dim_style()) // ○
                };
                let label = if sandbox.running_containers > 0 {
                    format!("Docker  {} containers", sandbox.running_containers)
                } else {
                    format!("Docker  {}", sandbox.status)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), style),
                    Span::styled(label, style),
                ]));
            }

            if let Some(ref secrets) = state.secrets_status {
                let (icon, style) = if secrets.vault_unlocked {
                    ("\u{1F513}", self.theme.success_style()) // 🔓
                } else {
                    ("\u{1F512}", self.theme.dim_style()) // 🔒
                };
                let label = format!("Secrets  {} stored", secrets.count);
                let status = if secrets.vault_unlocked {
                    "unlocked"
                } else {
                    "locked"
                };
                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), style),
                    Span::styled(label, self.theme.bold_style()),
                    Span::raw("  "),
                    Span::styled(status.to_string(), style),
                ]));
            }

            lines.push(Line::from(""));
        }

        // ── JOBS section ──────────────────────────────────────────────
        lines.push(self.render_section_header("JOBS", state.jobs.len(), width));

        if state.jobs.is_empty() {
            lines.push(Line::from(Span::styled(
                " (no jobs)",
                self.theme.dim_style(),
            )));
        }

        for job in &state.jobs {
            let style = self.job_status_style(job.status);
            let icon = job_icon(job.status);

            let uptime_secs = now
                .signed_duration_since(job.started_at)
                .num_seconds()
                .max(0) as u64;
            let uptime = format_uptime(uptime_secs);

            let name = truncate(&job.title, max_name_len);
            let padded_name = format!("{:<width$}", name, width = max_name_len);
            let status_text = format!("{}", job.status);

            lines.push(Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(padded_name, self.theme.bold_style()),
                Span::raw("  "),
                Span::styled(format!("{:<6}", status_text), style),
                Span::raw("  "),
                Span::styled(uptime, self.theme.dim_style()),
            ]));
        }

        // ── ROUTINES section ──────────────────────────────────────────
        lines.push(self.render_section_header("ROUTINES", state.routines.len(), width));

        if state.routines.is_empty() {
            lines.push(Line::from(Span::styled(
                " (no routines)",
                self.theme.dim_style(),
            )));
        }

        // Routines layout: " ◆ {name}  {trigger}  {on/off}"
        let routine_fixed: usize = 3 + 12 + 5; // icon + trigger + on/off + spacing
        let routine_name_len = width.saturating_sub(routine_fixed).max(4);

        for routine in &state.routines {
            let icon = "\u{25C6}"; // ◆
            let style = if routine.enabled {
                self.theme.accent_style()
            } else {
                self.theme.dim_style()
            };

            let name = truncate(&routine.name, routine_name_len);
            let padded_name = format!("{:<width$}", name, width = routine_name_len);
            let trigger = truncate(&routine.trigger_type, 10);
            let enabled_text = if routine.enabled { "on" } else { "off" };

            lines.push(Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(padded_name, self.theme.bold_style()),
                Span::raw("  "),
                Span::styled(format!("{:<10}", trigger), self.theme.dim_style()),
                Span::raw("  "),
                Span::styled(enabled_text.to_string(), style),
            ]));
        }

        // ── THREADS section ───────────────────────────────────────────
        lines.push(self.render_section_header("THREADS", state.threads.len(), width));

        if state.threads.is_empty() {
            lines.push(Line::from(Span::styled(
                " (no threads)",
                self.theme.dim_style(),
            )));
        }

        for thread in &state.threads {
            let style = self.thread_status_style(thread.status);
            let icon = thread_icon(thread.status);

            let uptime_secs = now
                .signed_duration_since(thread.started_at)
                .num_seconds()
                .max(0) as u64;
            let uptime = format_uptime(uptime_secs);

            let name = if thread.label.is_empty() {
                truncate(&thread.id, max_name_len)
            } else {
                truncate(&thread.label, max_name_len)
            };

            let padded_name = format!("{:<width$}", name, width = max_name_len);
            let status_text = format!("{}", thread.status);

            lines.push(Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(padded_name, self.theme.bold_style()),
                Span::raw("  "),
                Span::styled(format!("{:<6}", status_text), style),
                Span::raw("  "),
                Span::styled(uptime, self.theme.dim_style()),
            ]));
        }

        let visible: Vec<Line<'_>> = lines.into_iter().take(inner.height as usize).collect();
        let paragraph = ratatui::widgets::Paragraph::new(visible);
        paragraph.render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::widgets::{AppState, JobInfo, JobStatus, RoutineInfo, ThreadInfo, ThreadStatus};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn make_state() -> AppState {
        AppState::default()
    }

    fn render_to_buffer(widget: &ThreadListWidget, state: &AppState, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, state);
        buf
    }

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut text = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn empty_state_shows_all_sections() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let state = make_state();
        let buf = render_to_buffer(&widget, &state, 40, 10);
        let text = buffer_text(&buf);
        // With no data at all, the widget shows an empty-state message
        assert!(text.contains("No active jobs or routines"));
    }

    #[test]
    fn renders_jobs_section() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.jobs = vec![
            JobInfo {
                id: "j1".to_string(),
                title: "build-frontend".to_string(),
                status: JobStatus::Running,
                started_at: chrono::Utc::now() - chrono::Duration::seconds(180),
            },
            JobInfo {
                id: "j2".to_string(),
                title: "daily-sync".to_string(),
                status: JobStatus::Completed,
                started_at: chrono::Utc::now() - chrono::Duration::seconds(900),
            },
        ];
        let buf = render_to_buffer(&widget, &state, 50, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("JOBS (2)"));
        assert!(text.contains("build-frontend"));
        assert!(text.contains("running"));
        assert!(text.contains("daily-sync"));
        assert!(text.contains("done"));
    }

    #[test]
    fn renders_routines_section() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.routines = vec![RoutineInfo {
            id: "r1".to_string(),
            name: "issue-watch".to_string(),
            trigger_type: "github".to_string(),
            enabled: true,
            last_run: None,
            next_fire: None,
        }];
        let buf = render_to_buffer(&widget, &state, 50, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("ROUTINES (1)"));
        assert!(text.contains("issue-watch"));
        assert!(text.contains("github"));
        assert!(text.contains("on"));
    }

    #[test]
    fn renders_disabled_routine() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.routines = vec![RoutineInfo {
            id: "r1".to_string(),
            name: "backup".to_string(),
            trigger_type: "cron".to_string(),
            enabled: false,
            last_run: None,
            next_fire: None,
        }];
        let buf = render_to_buffer(&widget, &state, 50, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("off"));
    }

    #[test]
    fn renders_threads_section() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.threads = vec![ThreadInfo {
            id: "t1".to_string(),
            label: "main".to_string(),
            is_foreground: true,
            is_running: true,
            duration_secs: 120,
            status: ThreadStatus::Active,
            started_at: chrono::Utc::now() - chrono::Duration::seconds(120),
        }];
        let buf = render_to_buffer(&widget, &state, 50, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("THREADS (1)"));
        assert!(text.contains("main"));
        assert!(text.contains("active"));
        assert!(text.contains("\u{25CF}")); // ● icon
    }

    #[test]
    fn renders_all_three_sections() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let now = chrono::Utc::now();
        let mut state = make_state();
        state.jobs = vec![JobInfo {
            id: "j1".to_string(),
            title: "build".to_string(),
            status: JobStatus::Running,
            started_at: now - chrono::Duration::seconds(60),
        }];
        state.routines = vec![RoutineInfo {
            id: "r1".to_string(),
            name: "watch".to_string(),
            trigger_type: "event".to_string(),
            enabled: true,
            last_run: None,
            next_fire: None,
        }];
        state.threads = vec![ThreadInfo {
            id: "t1".to_string(),
            label: "main".to_string(),
            is_foreground: true,
            is_running: true,
            duration_secs: 30,
            status: ThreadStatus::Active,
            started_at: now - chrono::Duration::seconds(30),
        }];
        let buf = render_to_buffer(&widget, &state, 50, 15);
        let text = buffer_text(&buf);
        assert!(text.contains("JOBS (1)"));
        assert!(text.contains("build"));
        assert!(text.contains("ROUTINES (1)"));
        assert!(text.contains("watch"));
        assert!(text.contains("THREADS (1)"));
        assert!(text.contains("main"));
    }

    #[test]
    fn too_small_area_renders_nothing() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let state = make_state();
        let buf = render_to_buffer(&widget, &state, 3, 5);
        let text = buffer_text(&buf);
        assert!(!text.contains("JOBS"));
    }

    #[test]
    fn zero_height_renders_nothing() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.jobs = vec![JobInfo {
            id: "j1".to_string(),
            title: "test".to_string(),
            status: JobStatus::Running,
            started_at: chrono::Utc::now(),
        }];
        let buf = render_to_buffer(&widget, &state, 40, 0);
        let text = buffer_text(&buf);
        assert!(text.is_empty() || text.trim().is_empty());
    }

    #[test]
    fn job_status_display() {
        assert_eq!(format!("{}", JobStatus::Pending), "pending");
        assert_eq!(format!("{}", JobStatus::Running), "running");
        assert_eq!(format!("{}", JobStatus::Completed), "done");
        assert_eq!(format!("{}", JobStatus::Failed), "failed");
    }

    #[test]
    fn thread_status_display() {
        assert_eq!(format!("{}", ThreadStatus::Active), "active");
        assert_eq!(format!("{}", ThreadStatus::Idle), "idle");
        assert_eq!(format!("{}", ThreadStatus::Completed), "done");
        assert_eq!(format!("{}", ThreadStatus::Failed), "failed");
    }

    #[test]
    fn format_uptime_seconds() {
        assert_eq!(super::format_uptime(30), "30s");
    }

    #[test]
    fn format_uptime_minutes() {
        assert_eq!(super::format_uptime(150), "2m");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(super::format_uptime(3720), "1h 2m");
    }

    #[test]
    fn format_uptime_exact_hour() {
        assert_eq!(super::format_uptime(7200), "2h");
    }

    #[test]
    fn label_falls_back_to_id() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.threads = vec![ThreadInfo {
            id: "abc-def".to_string(),
            label: String::new(),
            is_foreground: false,
            is_running: true,
            duration_secs: 10,
            status: ThreadStatus::Active,
            started_at: chrono::Utc::now(),
        }];
        let buf = render_to_buffer(&widget, &state, 50, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("abc-def"));
    }

    #[test]
    fn renders_system_section_with_docker() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.sandbox_status = Some(crate::widgets::SandboxInfo {
            docker_available: true,
            running_containers: 2,
            status: "ready".to_string(),
        });
        let buf = render_to_buffer(&widget, &state, 50, 15);
        let text = buffer_text(&buf);
        assert!(text.contains("SYSTEM (1)"));
        assert!(text.contains("Docker"));
        assert!(text.contains("2 containers"));
    }

    #[test]
    fn renders_system_section_with_secrets() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.secrets_status = Some(crate::widgets::SecretsInfo {
            count: 5,
            vault_unlocked: true,
        });
        let buf = render_to_buffer(&widget, &state, 50, 15);
        let text = buffer_text(&buf);
        assert!(text.contains("SYSTEM (1)"));
        assert!(text.contains("Secrets"));
        assert!(text.contains("5 stored"));
        assert!(text.contains("unlocked"));
    }

    #[test]
    fn renders_system_section_with_both() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.sandbox_status = Some(crate::widgets::SandboxInfo {
            docker_available: false,
            running_containers: 0,
            status: "unavailable".to_string(),
        });
        state.secrets_status = Some(crate::widgets::SecretsInfo {
            count: 0,
            vault_unlocked: false,
        });
        let buf = render_to_buffer(&widget, &state, 50, 15);
        let text = buffer_text(&buf);
        assert!(text.contains("SYSTEM (2)"));
        assert!(text.contains("Docker"));
        assert!(text.contains("unavailable"));
        assert!(text.contains("Secrets"));
        assert!(text.contains("locked"));
    }

    #[test]
    fn no_system_section_without_data() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let state = make_state();
        let buf = render_to_buffer(&widget, &state, 50, 15);
        let text = buffer_text(&buf);
        assert!(!text.contains("SYSTEM"));
    }

    #[test]
    fn failed_job_shows_correct_icon() {
        let theme = Theme::dark();
        let widget = ThreadListWidget::new(theme);
        let mut state = make_state();
        state.jobs = vec![JobInfo {
            id: "j1".to_string(),
            title: "broken".to_string(),
            status: JobStatus::Failed,
            started_at: chrono::Utc::now() - chrono::Duration::seconds(30),
        }];
        let buf = render_to_buffer(&widget, &state, 50, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("\u{2717}")); // ✗ icon
        assert!(text.contains("failed"));
    }
}
