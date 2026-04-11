//! Dashboard widget: 2-column grid introspection panel.
//!
//! Layout:
//! ┌─ Title ──────────────────────────────────────────────┐
//! │ ◎ Dashboard  ·  model  ·  uptime  ·  cost           │
//! ├─ Summary ────────────────────────────────────────────┤
//! │ 79.9K tokens  ·  9 tools  ·  9 msgs  ·  142 memories│
//! ├─ Context Window (full width) ────────────────────────┤
//! │ [▓▓▓▓▓░░░░░░░░░░░░░░░]  30.7K / 128K (24%)         │
//! ├──────────────────────┬───────────────────────────────┤
//! │ Token Usage Per Turn │ Top Tools                     │
//! ├──────────────────────┼───────────────────────────────┤
//! │ System               │ Workspace                     │
//! ├──────────────────────┼───────────────────────────────┤
//! │ Jobs                 │ Engine Threads                │
//! ├──────────────────────┼───────────────────────────────┤
//! │ Skills               │ Learnings                     │
//! ├──────────────────────┴───────────────────────────────┤
//! │ Self-Learning                                        │
//! ├──────────────────────────────────────────────────────┤
//! │ Missions                                             │
//! └──────────────────────────────────────────────────────┘

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::{format_duration, format_tokens, truncate};
use crate::theme::Theme;

use std::collections::HashMap;

use super::thread_list::{format_uptime, job_icon, thread_icon, thread_type_tag};
use super::{
    AppState, DashboardPanel, JobStatus, MessageRole, ThreadStatus, ToolActivity, TuiWidget,
};

/// Sparkline bar characters from shortest to tallest.
const SPARK_CHARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub struct DashboardWidget {
    theme: Theme,
}

impl DashboardWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for DashboardWidget {
    fn id(&self) -> &str {
        "dashboard"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height < 6 || area.width < 30 {
            return;
        }

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // title
                Constraint::Length(1), // summary bar
                Constraint::Length(3), // context window
                Constraint::Length(8), // token usage | top tools
                Constraint::Length(6), // system | workspace
                Constraint::Length(6), // jobs | engine threads
                Constraint::Length(7), // skills | learnings
                Constraint::Length(5), // self-learning (full width)
                Constraint::Min(3),    // missions
            ])
            .split(area);

        self.render_title(sections[0], buf, state);
        self.render_summary_bar(sections[1], buf, state);
        self.render_context_panel(sections[2], buf, state);

        // 2-column rows
        let cols_upper = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[3]);
        self.render_token_panel(cols_upper[0], buf, state);
        self.render_tools_panel(cols_upper[1], buf, state);

        let cols_mid = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[4]);
        self.render_system_panel(cols_mid[0], buf, state);
        self.render_workspace_panel(cols_mid[1], buf, state);

        let cols_lower = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[5]);
        self.render_jobs_panel(cols_lower[0], buf, state);
        self.render_threads_panel(cols_lower[1], buf, state);

        let cols_neural = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[6]);
        self.render_skills_panel(cols_neural[0], buf, state);
        self.render_learnings_panel(cols_neural[1], buf, state);

        self.render_self_learning_panel(sections[7], buf, state);

        self.render_missions_panel(sections[8], buf, state);
    }
}

impl DashboardWidget {
    // ── Title bar ────────────────────────────────────────

    fn render_title(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let elapsed = chrono::Utc::now()
            .signed_duration_since(state.session_start)
            .num_seconds()
            .unsigned_abs();

        let line = Line::from(vec![
            Span::styled("  ◎ ", self.theme.accent_style()),
            Span::styled("Dashboard", self.theme.bold_accent_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(state.model.clone(), self.theme.dim_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(format_duration(elapsed), self.theme.dim_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(state.total_cost_usd.clone(), self.theme.dim_style()),
        ]);
        Paragraph::new(line).render(area, buf);
    }

    // ── Summary bar ──────────────────────────────────────

    fn render_summary_bar(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let total_tokens = state.total_input_tokens + state.total_output_tokens;
        let tool_count = state.recent_tools.len();
        let memory_count = if state.dashboard.total_memories > 0 {
            state.dashboard.total_memories
        } else {
            state.memory_count
        };

        let line = Line::from(vec![
            Span::styled("  ", self.theme.dim_style()),
            Span::styled(format_tokens(total_tokens), self.theme.bold_accent_style()),
            Span::styled(" tokens", self.theme.dim_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(format!("{tool_count}"), self.theme.bold_accent_style()),
            Span::styled(" tool calls", self.theme.dim_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(
                format!("{}", state.messages.len()),
                self.theme.bold_accent_style(),
            ),
            Span::styled(" messages", self.theme.dim_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(format!("{memory_count}"), self.theme.bold_accent_style()),
            Span::styled(" memories", self.theme.dim_style()),
        ]);
        Paragraph::new(line).render(area, buf);
    }

    // ── Context Window (full width) ──────────────────────

    fn render_context_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(
                " Context Window ",
                self.theme.bold_accent_style(),
            ))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width < 10 {
            return;
        }

        let total_tokens = state.total_input_tokens + state.total_output_tokens;
        if state.context_window == 0 {
            return;
        }

        let ratio = (total_tokens as f64 / state.context_window as f64).clamp(0.0, 1.0);
        let pct = (ratio * 100.0).round() as u64;
        // Use most of the available inner width for the bar
        let bar_width = (inner.width as usize).saturating_sub(30).max(10);
        let bar = capacity_bar(ratio, bar_width);
        let bar_style = if pct >= 90 {
            self.theme.error_style()
        } else if pct >= 70 {
            self.theme.warning_style()
        } else {
            self.theme.accent_style()
        };

        let detail = format!(
            " {} / {} ({pct}%)",
            format_tokens(total_tokens),
            format_tokens(state.context_window),
        );

        let line = Line::from(vec![
            Span::styled(" ", self.theme.dim_style()),
            Span::styled(bar, bar_style),
            Span::styled(detail, self.theme.dim_style()),
        ]);
        Paragraph::new(line).render(inner, buf);
    }

    // ── Token Usage Per Turn (left column) ───────────────

    fn render_token_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(
                " Token Usage ",
                self.theme.bold_accent_style(),
            ))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let dash = &state.dashboard;
        let mut lines: Vec<Line> = Vec::new();

        let sparkline_data: Vec<u64> = if !dash.token_sparkline.is_empty() {
            dash.token_sparkline.clone()
        } else {
            state
                .messages
                .iter()
                .filter_map(|m| {
                    m.cost_summary
                        .as_ref()
                        .map(|c| c.input_tokens + c.output_tokens)
                })
                .collect()
        };

        if sparkline_data.is_empty() {
            lines.push(Line::from(Span::styled(
                " No turn data yet",
                self.theme.dim_style(),
            )));
        } else {
            let sparkline = render_sparkline(&sparkline_data);
            lines.push(Line::from(vec![
                Span::styled(" ", self.theme.dim_style()),
                Span::styled(sparkline, self.theme.accent_style()),
            ]));

            // Min / avg / max stats
            let min_val = sparkline_data.iter().copied().min().unwrap_or(0);
            let max_val = sparkline_data.iter().copied().max().unwrap_or(0);
            let avg_val = if sparkline_data.is_empty() {
                0
            } else {
                sparkline_data.iter().sum::<u64>() / sparkline_data.len() as u64
            };

            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" min: ", self.theme.dim_style()),
                Span::styled(format_tokens(min_val), self.theme.accent_style()),
                Span::styled("  avg: ", self.theme.dim_style()),
                Span::styled(format_tokens(avg_val), self.theme.accent_style()),
                Span::styled("  max: ", self.theme.dim_style()),
                Span::styled(format_tokens(max_val), self.theme.accent_style()),
            ]));

            // Input vs output breakdown
            lines.push(Line::from(vec![
                Span::styled(" in: ", self.theme.dim_style()),
                Span::styled(
                    format_tokens(state.total_input_tokens),
                    self.theme.dim_style(),
                ),
                Span::styled("  out: ", self.theme.dim_style()),
                Span::styled(
                    format_tokens(state.total_output_tokens),
                    self.theme.dim_style(),
                ),
            ]));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Top Tools (right column) ─────────────────────────

    fn render_tools_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Top Tools ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let dash = &state.dashboard;
        let tool_freq: Vec<(String, usize)> = if !dash.tool_frequency.is_empty() {
            dash.tool_frequency.clone()
        } else {
            compute_tool_frequency(&state.recent_tools)
        };

        let mut lines: Vec<Line> = Vec::new();

        if tool_freq.is_empty() {
            lines.push(Line::from(Span::styled(
                " No tool calls yet",
                self.theme.dim_style(),
            )));
        } else {
            let max_count = tool_freq.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            let bar_max_width = (inner.width as usize).saturating_sub(22).max(4);

            for (name, count) in tool_freq.iter().take(inner.height as usize) {
                let bar_len =
                    ((*count as f64 / max_count as f64) * bar_max_width as f64).ceil() as usize;
                let bar: String = "▓".repeat(bar_len.max(1));

                lines.push(Line::from(vec![
                    Span::styled(format!(" {name:<14} "), self.theme.dim_style()),
                    Span::styled(bar, self.theme.accent_style()),
                    Span::styled(format!(" {count}"), self.theme.dim_style()),
                ]));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── System (left column) ─────────────────────────────

    fn render_system_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" System ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let session_id = state.session_start.format("%Y%m%d_%H%M%S").to_string();
        let ctx_label = format!("{} tokens", format_tokens(state.context_window));

        let mut lines = vec![
            Line::from(vec![
                Span::styled(" Model    ", self.theme.dim_style()),
                Span::styled(state.model.clone(), self.theme.accent_style()),
            ]),
            Line::from(vec![
                Span::styled(" Context  ", self.theme.dim_style()),
                Span::styled(ctx_label, self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Session  ", self.theme.dim_style()),
                Span::styled(session_id, self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Cost     ", self.theme.dim_style()),
                Span::styled(state.total_cost_usd.clone(), self.theme.accent_style()),
            ]),
        ];

        // Docker status
        if let Some(ref sandbox) = state.sandbox_status {
            let (icon, style) = if sandbox.docker_available {
                ("\u{25CF}", self.theme.success_style()) // ●
            } else {
                ("\u{25CB}", self.theme.dim_style()) // ○
            };
            let label = if sandbox.running_containers > 0 {
                format!("{icon} Docker: {} containers", sandbox.running_containers)
            } else {
                format!("{icon} Docker: {}", sandbox.status)
            };
            lines.push(Line::from(Span::styled(format!(" {label}"), style)));
        }

        // Secrets vault
        if let Some(ref secrets) = state.secrets_status {
            let (icon, style) = if secrets.vault_unlocked {
                ("\u{1F513}", self.theme.success_style())
            } else {
                ("\u{1F512}", self.theme.dim_style())
            };
            let status = if secrets.vault_unlocked {
                "unlocked"
            } else {
                "locked"
            };
            lines.push(Line::from(Span::styled(
                format!(" {icon} Secrets: {} stored, {status}", secrets.count),
                style,
            )));
        }

        // CostGuard budget
        if let Some(ref cg) = state.cost_guard {
            let style = if cg.limit_reached {
                self.theme.error_style()
            } else {
                self.theme.dim_style()
            };
            let budget_text = if let Some(ref budget) = cg.session_budget_usd {
                format!("{}/{budget}", cg.spent_usd)
            } else {
                cg.spent_usd.clone()
            };
            lines.push(Line::from(Span::styled(
                format!(" Budget   {budget_text}"),
                style,
            )));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Workspace (right column) ─────────────────────────

    fn render_workspace_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Workspace ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let dash = &state.dashboard;
        let memory_count = if dash.total_memories > 0 {
            dash.total_memories
        } else {
            state.memory_count
        };
        let identity_files = if !dash.identity_files.is_empty() {
            &dash.identity_files
        } else {
            &state.identity_files
        };

        let max_path_len = (inner.width as usize).saturating_sub(11);
        let path_display = if state.workspace_path.len() > max_path_len && max_path_len > 3 {
            format!(
                "...{}",
                &state.workspace_path[state.workspace_path.len() - max_path_len + 3..]
            )
        } else {
            state.workspace_path.clone()
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(" Path     ", self.theme.dim_style()),
                Span::styled(path_display, self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Memories ", self.theme.dim_style()),
                Span::styled(format!("{memory_count}"), self.theme.accent_style()),
            ]),
        ];

        if !identity_files.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" Identity ", self.theme.dim_style()),
                Span::styled(identity_files.join(", "), self.theme.success_style()),
            ]));
        }

        if let Some((total, custom, builtin)) = dash.skills_summary {
            lines.push(Line::from(vec![
                Span::styled(" Skills   ", self.theme.dim_style()),
                Span::styled(format!("{total}"), self.theme.accent_style()),
                Span::styled(
                    format!(" ({custom} custom, {builtin} built-in)"),
                    self.theme.dim_style(),
                ),
            ]));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Jobs (left column) ────────────────────────────────

    fn render_jobs_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Jobs ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let mut lines: Vec<Line> = Vec::new();
        let now = chrono::Utc::now();
        let max_name = (inner.width as usize).saturating_sub(16).max(4);

        if state.jobs.is_empty() {
            lines.push(Line::from(Span::styled(
                " No active jobs",
                self.theme.dim_style(),
            )));
        } else {
            for job in state.jobs.iter().take(inner.height as usize) {
                let icon = job_icon(job.status);
                let style = match job.status {
                    JobStatus::Running => self.theme.accent_style(),
                    JobStatus::Completed => self.theme.success_style(),
                    JobStatus::Failed => self.theme.error_style(),
                    JobStatus::Pending => self.theme.dim_style(),
                };
                let uptime_secs = now
                    .signed_duration_since(job.started_at)
                    .num_seconds()
                    .max(0) as u64;
                let name = truncate(&job.title, max_name);

                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), style),
                    Span::styled(name, self.theme.bold_style()),
                    Span::styled(format!("  {}", job.status), style),
                    Span::styled(
                        format!("  {}", format_uptime(uptime_secs)),
                        self.theme.dim_style(),
                    ),
                ]));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Engine Threads (right column) ────────────────────

    fn render_threads_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Threads ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let mut lines: Vec<Line> = Vec::new();
        let now = chrono::Utc::now();
        let max_goal = (inner.width as usize).saturating_sub(20).max(4);

        if state.engine_threads.is_empty() {
            lines.push(Line::from(Span::styled(
                " No active threads",
                self.theme.dim_style(),
            )));
        } else {
            for thread in state.engine_threads.iter().take(inner.height as usize) {
                let icon = thread_icon(thread.status);
                let style = match thread.status {
                    ThreadStatus::Active => self.theme.accent_style(),
                    ThreadStatus::Idle => self.theme.dim_style(),
                    ThreadStatus::Completed => self.theme.success_style(),
                    ThreadStatus::Failed => self.theme.error_style(),
                };
                let tag = thread_type_tag(&thread.thread_type);
                let goal = truncate(&thread.goal, max_goal);
                let uptime = thread
                    .started_at
                    .map(
                        |s| format_uptime(now.signed_duration_since(s).num_seconds().max(0) as u64),
                    )
                    .unwrap_or_else(|| "?".to_string());

                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), style),
                    Span::styled(format!("{tag} "), self.theme.dim_style()),
                    Span::styled(goal, self.theme.bold_style()),
                    Span::styled(format!("  {uptime}"), self.theme.dim_style()),
                ]));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Missions (full width, bottom) ────────────────────

    fn render_missions_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Missions ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let routines = if !state.dashboard.routines.is_empty() {
            &state.dashboard.routines
        } else {
            &state.routines
        };

        let mut lines: Vec<Line> = Vec::new();

        if routines.is_empty() {
            lines.push(Line::from(Span::styled(
                " No missions configured",
                self.theme.dim_style(),
            )));
        } else {
            for r in routines.iter().take(inner.height as usize) {
                let icon = if r.enabled { "▶" } else { "⏸" };
                let icon_style = if r.enabled {
                    self.theme.success_style()
                } else {
                    self.theme.dim_style()
                };
                let mut spans = vec![
                    Span::styled(format!(" {icon} "), icon_style),
                    Span::styled(format!("{:<20}", r.name), self.theme.accent_style()),
                    Span::styled(format!(" {:<12}", r.trigger_type), self.theme.dim_style()),
                ];
                if let Some(ref next) = r.next_fire {
                    spans.push(Span::styled(
                        format!(" next: {next}"),
                        self.theme.dim_style(),
                    ));
                }
                if let Some(ref last) = r.last_run {
                    spans.push(Span::styled(
                        format!(" last: {last}"),
                        self.theme.dim_style(),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Skills (left column) ─────────────────────────────

    fn render_skills_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Skills ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let dash = &state.dashboard;
        let mut lines: Vec<Line> = Vec::new();

        // Use dashboard skills_summary if available, else compute from welcome_skills
        let (total, custom, builtin) = if let Some(summary) = dash.skills_summary {
            summary
        } else {
            let builtin_categories = [
                "memory", "file", "browser", "shell", "http", "json", "time", "echo",
            ];
            let mut custom_count = 0usize;
            let mut builtin_count = 0usize;
            for cat in &state.welcome_skills {
                if builtin_categories.contains(&cat.name.to_lowercase().as_str()) {
                    builtin_count += cat.skills.len();
                } else {
                    custom_count += cat.skills.len();
                }
            }
            (custom_count + builtin_count, custom_count, builtin_count)
        };

        // Custom skills row
        let custom_names: Vec<&str> = state
            .welcome_skills
            .iter()
            .filter(|c| {
                ![
                    "memory", "file", "browser", "shell", "http", "json", "time", "echo",
                ]
                .contains(&c.name.to_lowercase().as_str())
            })
            .flat_map(|c| c.skills.iter().map(|s| s.as_str()))
            .collect();

        lines.push(Line::from(vec![
            Span::styled(" \u{250C} custom ", self.theme.accent_style()),
            Span::styled(format!("{}", custom), self.theme.bold_accent_style()),
        ]));

        if !custom_names.is_empty() {
            let max_width = (inner.width as usize).saturating_sub(4);
            let display = join_truncated(&custom_names, max_width);
            lines.push(Line::from(vec![
                Span::styled(" \u{2502} ", self.theme.dim_style()),
                Span::styled(display, self.theme.dim_style()),
            ]));
        }

        lines.push(Line::from(vec![
            Span::styled(" \u{2514} built-in ", self.theme.dim_style()),
            Span::styled(format!("{builtin}"), self.theme.dim_style()),
        ]));

        // Activated skills count
        if !state.activated_skills.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                format!(
                    " \u{26A1} activated: {} this session",
                    state.activated_skills.len()
                ),
                self.theme.success_style(),
            )]));
        } else {
            lines.push(Line::from(Span::styled(
                format!(" {total} total skills"),
                self.theme.dim_style(),
            )));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Learnings (right column) ─────────────────────────

    fn render_learnings_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(" Learnings ", self.theme.bold_accent_style()))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let dash = &state.dashboard;
        let memory_count = if dash.total_memories > 0 {
            dash.total_memories
        } else {
            state.memory_count
        };
        let identity_files = if !dash.identity_files.is_empty() {
            &dash.identity_files
        } else {
            &state.identity_files
        };

        let mut lines: Vec<Line> = Vec::new();

        // Memory count
        lines.push(Line::from(vec![
            Span::styled(" \u{25CF} memories ", self.theme.dim_style()),
            Span::styled(format!("{memory_count}"), self.theme.bold_accent_style()),
        ]));

        // Identity files
        if !identity_files.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" \u{25CB} identity ", self.theme.dim_style()),
                Span::styled(identity_files.join(", "), self.theme.success_style()),
            ]));
        }

        // Memory categories from dashboard data
        if !dash.memory_categories.is_empty() {
            lines.push(Line::from(Span::styled(
                " \u{2500} categories \u{2500}",
                self.theme.dim_style(),
            )));
            let cats: Vec<String> = dash
                .memory_categories
                .iter()
                .take(4)
                .map(|(name, count)| format!("{name}: {count}"))
                .collect();
            lines.push(Line::from(vec![
                Span::styled("   ", self.theme.dim_style()),
                Span::styled(cats.join("  "), self.theme.dim_style()),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                " awaiting introspection",
                self.theme.dim_style(),
            )));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Self-Learning (full width) ───────────────────────

    fn render_self_learning_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .title(Span::styled(
                " Self-Learning ",
                self.theme.bold_accent_style(),
            ))
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        // Compute turn count and avg tokens from messages
        let turn_count = state
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count();

        let per_turn_tokens: Vec<u64> = state
            .messages
            .iter()
            .filter_map(|m| {
                m.cost_summary
                    .as_ref()
                    .map(|c| c.input_tokens + c.output_tokens)
            })
            .collect();

        let avg_tokens = if per_turn_tokens.is_empty() {
            0
        } else {
            per_turn_tokens.iter().sum::<u64>() / per_turn_tokens.len() as u64
        };

        // First line: turns, avg tokens, sparkline
        let mut spans = vec![
            Span::styled(" turns: ", self.theme.dim_style()),
            Span::styled(format!("{turn_count}"), self.theme.bold_accent_style()),
            Span::styled("   avg tokens/turn: ", self.theme.dim_style()),
            Span::styled(format_tokens(avg_tokens), self.theme.bold_accent_style()),
        ];

        if !per_turn_tokens.is_empty() {
            let sparkline = render_sparkline(&per_turn_tokens);
            spans.push(Span::styled("   trend: ", self.theme.dim_style()));
            spans.push(Span::styled(sparkline, self.theme.accent_style()));
        }

        lines.push(Line::from(spans));

        // Second line: tool repertoire (top 4) + skills activated
        let tool_freq = if !state.dashboard.tool_frequency.is_empty() {
            state.dashboard.tool_frequency.clone()
        } else {
            compute_tool_frequency(&state.recent_tools)
        };

        if !tool_freq.is_empty() {
            let max_count = tool_freq.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            let mut repo_spans: Vec<Span> =
                vec![Span::styled(" tool repertoire: ", self.theme.dim_style())];
            for (name, count) in tool_freq.iter().take(4) {
                let bar_len = ((*count as f64 / max_count as f64) * 4.0).ceil() as usize;
                let bar: String = "\u{2593}".repeat(bar_len.max(1));
                repo_spans.push(Span::styled(format!("{name} "), self.theme.dim_style()));
                repo_spans.push(Span::styled(format!("{bar}  "), self.theme.accent_style()));
            }
            lines.push(Line::from(repo_spans));
        }

        // Third line: skills activated count
        if !state.activated_skills.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" skills activated: ", self.theme.dim_style()),
                Span::styled(
                    format!("{} this session", state.activated_skills.len()),
                    self.theme.success_style(),
                ),
            ]));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    // ── Expanded Panel Modal ────────────────────────────

    /// Render an expanded dashboard panel as a centered modal overlay.
    pub fn render_expanded_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let Some(ref modal) = state.expanded_dashboard_panel else {
            return;
        };

        let width = (area.width * 3 / 4)
            .max(40)
            .min(area.width.saturating_sub(4));
        let height = (area.height * 3 / 4)
            .max(10)
            .min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        Clear.render(modal_area, buf);

        let title = match modal.panel {
            DashboardPanel::TokenUsage => " Token Usage (expanded) ",
            DashboardPanel::TopTools => " Top Tools (expanded) ",
            DashboardPanel::System => " System (expanded) ",
            DashboardPanel::Workspace => " Workspace (expanded) ",
            DashboardPanel::Jobs => " Jobs (expanded) ",
            DashboardPanel::Threads => " Threads (expanded) ",
            DashboardPanel::Skills => " Skills (expanded) ",
            DashboardPanel::Learnings => " Learnings (expanded) ",
            DashboardPanel::SelfLearning => " Self-Learning (expanded) ",
            DashboardPanel::Missions => " Missions (expanded) ",
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.accent_style())
            .title(Span::styled(title, self.theme.bold_accent_style()));
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if inner.height == 0 {
            return;
        }

        let lines = match modal.panel {
            DashboardPanel::TokenUsage => self.expanded_token_lines(state, inner),
            DashboardPanel::TopTools => self.expanded_tools_lines(state, inner),
            DashboardPanel::System => self.expanded_system_lines(state),
            DashboardPanel::Workspace => self.expanded_workspace_lines(state, inner),
            DashboardPanel::Jobs => self.expanded_jobs_lines(state, inner),
            DashboardPanel::Threads => self.expanded_threads_lines(state, inner),
            DashboardPanel::Skills => self.expanded_skills_lines(state, inner),
            DashboardPanel::Learnings => self.expanded_learnings_lines(state),
            DashboardPanel::SelfLearning => self.expanded_self_learning_lines(state),
            DashboardPanel::Missions => self.expanded_missions_lines(state, inner),
        };

        // Add scroll hint at the bottom
        let total_lines = lines.len();
        let mut all_lines = lines;
        if total_lines > inner.height as usize {
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(Span::styled(
                " \u{2191}\u{2193} scroll  Esc close",
                self.theme.dim_style(),
            )));
        }

        Paragraph::new(all_lines)
            .scroll((modal.scroll, 0))
            .render(inner, buf);
    }

    fn expanded_token_lines(&self, state: &AppState, _inner: Rect) -> Vec<Line<'static>> {
        let dash = &state.dashboard;
        let mut lines: Vec<Line> = Vec::new();

        let sparkline_data: Vec<u64> = if !dash.token_sparkline.is_empty() {
            dash.token_sparkline.clone()
        } else {
            state
                .messages
                .iter()
                .filter_map(|m| {
                    m.cost_summary
                        .as_ref()
                        .map(|c| c.input_tokens + c.output_tokens)
                })
                .collect()
        };

        if sparkline_data.is_empty() {
            lines.push(Line::from(Span::styled(
                " No turn data yet",
                self.theme.dim_style(),
            )));
        } else {
            let sparkline = render_sparkline(&sparkline_data);
            lines.push(Line::from(vec![
                Span::styled(" ", self.theme.dim_style()),
                Span::styled(sparkline, self.theme.accent_style()),
            ]));
            lines.push(Line::from(""));

            let min_val = sparkline_data.iter().copied().min().unwrap_or(0);
            let max_val = sparkline_data.iter().copied().max().unwrap_or(0);
            let avg_val = sparkline_data.iter().sum::<u64>() / sparkline_data.len() as u64;

            lines.push(Line::from(vec![
                Span::styled(" min: ", self.theme.dim_style()),
                Span::styled(format_tokens(min_val), self.theme.accent_style()),
                Span::styled("  avg: ", self.theme.dim_style()),
                Span::styled(format_tokens(avg_val), self.theme.accent_style()),
                Span::styled("  max: ", self.theme.dim_style()),
                Span::styled(format_tokens(max_val), self.theme.accent_style()),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" in: ", self.theme.dim_style()),
                Span::styled(
                    format_tokens(state.total_input_tokens),
                    self.theme.dim_style(),
                ),
                Span::styled("  out: ", self.theme.dim_style()),
                Span::styled(
                    format_tokens(state.total_output_tokens),
                    self.theme.dim_style(),
                ),
            ]));
            lines.push(Line::from(""));

            // Per-turn breakdown
            lines.push(Line::from(Span::styled(
                " Per-turn breakdown:",
                self.theme.bold_accent_style(),
            )));
            for (i, &tokens) in sparkline_data.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::styled(format!("  Turn {:<4} ", i + 1), self.theme.dim_style()),
                    Span::styled(format_tokens(tokens), self.theme.accent_style()),
                ]));
            }
        }

        lines
    }

    fn expanded_tools_lines(&self, state: &AppState, inner: Rect) -> Vec<Line<'static>> {
        let dash = &state.dashboard;
        let tool_freq: Vec<(String, usize)> = if !dash.tool_frequency.is_empty() {
            dash.tool_frequency.clone()
        } else {
            compute_tool_frequency(&state.recent_tools)
        };

        let mut lines: Vec<Line> = Vec::new();

        if tool_freq.is_empty() {
            lines.push(Line::from(Span::styled(
                " No tool calls yet",
                self.theme.dim_style(),
            )));
        } else {
            let max_count = tool_freq.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            let bar_max_width = (inner.width as usize).saturating_sub(22).max(4);

            for (name, count) in &tool_freq {
                let bar_len =
                    ((*count as f64 / max_count as f64) * bar_max_width as f64).ceil() as usize;
                let bar: String = "\u{2593}".repeat(bar_len.max(1));

                lines.push(Line::from(vec![
                    Span::styled(format!(" {name:<14} "), self.theme.dim_style()),
                    Span::styled(bar, self.theme.accent_style()),
                    Span::styled(format!(" {count}"), self.theme.dim_style()),
                ]));
            }
        }

        lines
    }

    fn expanded_system_lines(&self, state: &AppState) -> Vec<Line<'static>> {
        let session_id = state.session_start.format("%Y%m%d_%H%M%S").to_string();
        let ctx_label = format!("{} tokens", format_tokens(state.context_window));
        let elapsed = chrono::Utc::now()
            .signed_duration_since(state.session_start)
            .num_seconds()
            .unsigned_abs();

        let mut lines = vec![
            Line::from(vec![
                Span::styled(" Model    ", self.theme.dim_style()),
                Span::styled(state.model.clone(), self.theme.accent_style()),
            ]),
            Line::from(vec![
                Span::styled(" Context  ", self.theme.dim_style()),
                Span::styled(ctx_label, self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Session  ", self.theme.dim_style()),
                Span::styled(session_id, self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Uptime   ", self.theme.dim_style()),
                Span::styled(format_duration(elapsed), self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Cost     ", self.theme.dim_style()),
                Span::styled(state.total_cost_usd.clone(), self.theme.accent_style()),
            ]),
            Line::from(vec![
                Span::styled(" Tokens   ", self.theme.dim_style()),
                Span::styled(
                    format!(
                        "in: {}  out: {}",
                        format_tokens(state.total_input_tokens),
                        format_tokens(state.total_output_tokens)
                    ),
                    self.theme.dim_style(),
                ),
            ]),
        ];

        if let Some(ref sandbox) = state.sandbox_status {
            let (icon, style) = if sandbox.docker_available {
                ("\u{25CF}", self.theme.success_style())
            } else {
                ("\u{25CB}", self.theme.dim_style())
            };
            let label = if sandbox.running_containers > 0 {
                format!("{icon} Docker: {} containers", sandbox.running_containers)
            } else {
                format!("{icon} Docker: {}", sandbox.status)
            };
            lines.push(Line::from(Span::styled(format!(" {label}"), style)));
        }

        if let Some(ref secrets) = state.secrets_status {
            let (icon, style) = if secrets.vault_unlocked {
                ("\u{1F513}", self.theme.success_style())
            } else {
                ("\u{1F512}", self.theme.dim_style())
            };
            let status = if secrets.vault_unlocked {
                "unlocked"
            } else {
                "locked"
            };
            lines.push(Line::from(Span::styled(
                format!(" {icon} Secrets: {} stored, {status}", secrets.count),
                style,
            )));
        }

        if let Some(ref cg) = state.cost_guard {
            let style = if cg.limit_reached {
                self.theme.error_style()
            } else {
                self.theme.dim_style()
            };
            let budget_text = if let Some(ref budget) = cg.session_budget_usd {
                format!("{}/{budget}", cg.spent_usd)
            } else {
                cg.spent_usd.clone()
            };
            lines.push(Line::from(Span::styled(
                format!(" Budget   {budget_text}"),
                style,
            )));
        }

        lines
    }

    fn expanded_workspace_lines(&self, state: &AppState, _inner: Rect) -> Vec<Line<'static>> {
        let dash = &state.dashboard;
        let memory_count = if dash.total_memories > 0 {
            dash.total_memories
        } else {
            state.memory_count
        };
        let identity_files = if !dash.identity_files.is_empty() {
            &dash.identity_files
        } else {
            &state.identity_files
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(" Path     ", self.theme.dim_style()),
                Span::styled(state.workspace_path.clone(), self.theme.dim_style()),
            ]),
            Line::from(vec![
                Span::styled(" Memories ", self.theme.dim_style()),
                Span::styled(format!("{memory_count}"), self.theme.accent_style()),
            ]),
        ];

        if !identity_files.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" Identity ", self.theme.dim_style()),
                Span::styled(identity_files.join(", "), self.theme.success_style()),
            ]));
        }

        if let Some((total, custom, builtin)) = dash.skills_summary {
            lines.push(Line::from(vec![
                Span::styled(" Skills   ", self.theme.dim_style()),
                Span::styled(format!("{total}"), self.theme.accent_style()),
                Span::styled(
                    format!(" ({custom} custom, {builtin} built-in)"),
                    self.theme.dim_style(),
                ),
            ]));
        }

        // Memory categories
        if !dash.memory_categories.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Memory categories:",
                self.theme.bold_accent_style(),
            )));
            for (name, count) in &dash.memory_categories {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {name}: "), self.theme.dim_style()),
                    Span::styled(format!("{count}"), self.theme.accent_style()),
                ]));
            }
        }

        lines
    }

    fn expanded_jobs_lines(&self, state: &AppState, inner: Rect) -> Vec<Line<'static>> {
        let mut lines: Vec<Line> = Vec::new();
        let now = chrono::Utc::now();
        let max_name = (inner.width as usize).saturating_sub(16).max(4);

        if state.jobs.is_empty() {
            lines.push(Line::from(Span::styled(
                " No active jobs",
                self.theme.dim_style(),
            )));
        } else {
            for job in &state.jobs {
                let icon = job_icon(job.status);
                let style = match job.status {
                    JobStatus::Running => self.theme.accent_style(),
                    JobStatus::Completed => self.theme.success_style(),
                    JobStatus::Failed => self.theme.error_style(),
                    JobStatus::Pending => self.theme.dim_style(),
                };
                let uptime_secs = now
                    .signed_duration_since(job.started_at)
                    .num_seconds()
                    .max(0) as u64;
                let name = truncate(&job.title, max_name);

                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), style),
                    Span::styled(name, self.theme.bold_style()),
                    Span::styled(format!("  {}", job.status), style),
                    Span::styled(
                        format!("  {}", format_uptime(uptime_secs)),
                        self.theme.dim_style(),
                    ),
                ]));
            }
        }

        lines
    }

    fn expanded_threads_lines(&self, state: &AppState, inner: Rect) -> Vec<Line<'static>> {
        let mut lines: Vec<Line> = Vec::new();
        let now = chrono::Utc::now();
        let max_goal = (inner.width as usize).saturating_sub(20).max(4);

        if state.engine_threads.is_empty() {
            lines.push(Line::from(Span::styled(
                " No active threads",
                self.theme.dim_style(),
            )));
        } else {
            for thread in &state.engine_threads {
                let icon = thread_icon(thread.status);
                let style = match thread.status {
                    ThreadStatus::Active => self.theme.accent_style(),
                    ThreadStatus::Idle => self.theme.dim_style(),
                    ThreadStatus::Completed => self.theme.success_style(),
                    ThreadStatus::Failed => self.theme.error_style(),
                };
                let tag = thread_type_tag(&thread.thread_type);
                let goal = truncate(&thread.goal, max_goal);
                let uptime = thread
                    .started_at
                    .map(
                        |s| format_uptime(now.signed_duration_since(s).num_seconds().max(0) as u64),
                    )
                    .unwrap_or_else(|| "?".to_string());

                lines.push(Line::from(vec![
                    Span::styled(format!(" {icon} "), style),
                    Span::styled(format!("{tag} "), self.theme.dim_style()),
                    Span::styled(goal, self.theme.bold_style()),
                    Span::styled(format!("  {uptime}"), self.theme.dim_style()),
                ]));
            }
        }

        lines
    }

    fn expanded_skills_lines(&self, state: &AppState, inner: Rect) -> Vec<Line<'static>> {
        let dash = &state.dashboard;
        let mut lines: Vec<Line> = Vec::new();

        let builtin_categories = [
            "memory", "file", "browser", "shell", "http", "json", "time", "echo",
        ];

        // Custom skills
        let custom_cats: Vec<&super::SkillCategory> = state
            .welcome_skills
            .iter()
            .filter(|c| !builtin_categories.contains(&c.name.to_lowercase().as_str()))
            .collect();

        let custom_count: usize = custom_cats.iter().map(|c| c.skills.len()).sum();
        let builtin_count: usize = state
            .welcome_skills
            .iter()
            .filter(|c| builtin_categories.contains(&c.name.to_lowercase().as_str()))
            .map(|c| c.skills.len())
            .sum();

        if let Some((total, custom, builtin)) = dash.skills_summary {
            lines.push(Line::from(vec![Span::styled(
                format!(" {total} total skills ({custom} custom, {builtin} built-in)"),
                self.theme.bold_accent_style(),
            )]));
        } else {
            let total = custom_count + builtin_count;
            lines.push(Line::from(vec![Span::styled(
                format!(" {total} total skills ({custom_count} custom, {builtin_count} built-in)"),
                self.theme.bold_accent_style(),
            )]));
        }
        lines.push(Line::from(""));

        // Custom categories with all skill names
        if !custom_cats.is_empty() {
            lines.push(Line::from(Span::styled(
                " Custom Skills:",
                self.theme.accent_style(),
            )));
            for cat in &custom_cats {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {} ", cat.name), self.theme.accent_style()),
                    Span::styled(format!("({})", cat.skills.len()), self.theme.dim_style()),
                ]));
                // Show all skills in this category
                let max_width = (inner.width as usize).saturating_sub(6);
                let mut current_line = String::new();
                for (i, skill) in cat.skills.iter().enumerate() {
                    let sep = if i > 0 { "  " } else { "" };
                    if current_line.len() + sep.len() + skill.len() > max_width
                        && !current_line.is_empty()
                    {
                        lines.push(Line::from(vec![
                            Span::styled("     ", self.theme.dim_style()),
                            Span::styled(current_line.clone(), self.theme.dim_style()),
                        ]));
                        current_line = skill.clone();
                    } else {
                        current_line.push_str(sep);
                        current_line.push_str(skill);
                    }
                }
                if !current_line.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("     ", self.theme.dim_style()),
                        Span::styled(current_line, self.theme.dim_style()),
                    ]));
                }
            }
        }

        // Built-in categories
        let builtin_cats: Vec<&super::SkillCategory> = state
            .welcome_skills
            .iter()
            .filter(|c| builtin_categories.contains(&c.name.to_lowercase().as_str()))
            .collect();

        if !builtin_cats.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Built-in Skills:",
                self.theme.dim_style(),
            )));
            for cat in &builtin_cats {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {} ", cat.name), self.theme.dim_style()),
                    Span::styled(format!("({})", cat.skills.len()), self.theme.dim_style()),
                ]));
            }
        }

        // Activated skills
        if !state.activated_skills.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!(
                    " \u{26A1} Activated this session: {}",
                    state.activated_skills.len()
                ),
                self.theme.success_style(),
            )]));
            let max_width = (inner.width as usize).saturating_sub(4);
            let mut current_line = String::new();
            for (i, name) in state.activated_skills.iter().enumerate() {
                let sep = if i > 0 { "  " } else { "" };
                if current_line.len() + sep.len() + name.len() > max_width
                    && !current_line.is_empty()
                {
                    lines.push(Line::from(vec![
                        Span::styled("   ", self.theme.dim_style()),
                        Span::styled(current_line.clone(), self.theme.success_style()),
                    ]));
                    current_line = name.clone();
                } else {
                    current_line.push_str(sep);
                    current_line.push_str(name);
                }
            }
            if !current_line.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("   ", self.theme.dim_style()),
                    Span::styled(current_line, self.theme.success_style()),
                ]));
            }
        }

        lines
    }

    fn expanded_learnings_lines(&self, state: &AppState) -> Vec<Line<'static>> {
        let dash = &state.dashboard;
        let memory_count = if dash.total_memories > 0 {
            dash.total_memories
        } else {
            state.memory_count
        };
        let identity_files = if !dash.identity_files.is_empty() {
            &dash.identity_files
        } else {
            &state.identity_files
        };

        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(vec![
            Span::styled(" \u{25CF} memories ", self.theme.dim_style()),
            Span::styled(format!("{memory_count}"), self.theme.bold_accent_style()),
        ]));

        if !identity_files.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" \u{25CB} identity ", self.theme.dim_style()),
                Span::styled(identity_files.join(", "), self.theme.success_style()),
            ]));
        }

        if !dash.memory_categories.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Memory categories:",
                self.theme.bold_accent_style(),
            )));
            for (name, count) in &dash.memory_categories {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {name}: "), self.theme.dim_style()),
                    Span::styled(format!("{count}"), self.theme.accent_style()),
                ]));
            }
        }

        if !dash.session_history.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Session history:",
                self.theme.bold_accent_style(),
            )));
            for session in &dash.session_history {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {} ", session.label), self.theme.dim_style()),
                    Span::styled(
                        format!(
                            "{} msgs, {} tools, {} tokens",
                            session.message_count,
                            session.tool_calls,
                            format_tokens(session.tokens)
                        ),
                        self.theme.dim_style(),
                    ),
                ]));
            }
        }

        lines
    }

    fn expanded_self_learning_lines(&self, state: &AppState) -> Vec<Line<'static>> {
        let mut lines: Vec<Line> = Vec::new();

        let turn_count = state
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count();

        let per_turn_tokens: Vec<u64> = state
            .messages
            .iter()
            .filter_map(|m| {
                m.cost_summary
                    .as_ref()
                    .map(|c| c.input_tokens + c.output_tokens)
            })
            .collect();

        let avg_tokens = if per_turn_tokens.is_empty() {
            0
        } else {
            per_turn_tokens.iter().sum::<u64>() / per_turn_tokens.len() as u64
        };

        lines.push(Line::from(vec![
            Span::styled(" turns: ", self.theme.dim_style()),
            Span::styled(format!("{turn_count}"), self.theme.bold_accent_style()),
            Span::styled("   avg tokens/turn: ", self.theme.dim_style()),
            Span::styled(format_tokens(avg_tokens), self.theme.bold_accent_style()),
        ]));

        if !per_turn_tokens.is_empty() {
            let sparkline = render_sparkline(&per_turn_tokens);
            lines.push(Line::from(vec![
                Span::styled(" trend: ", self.theme.dim_style()),
                Span::styled(sparkline, self.theme.accent_style()),
            ]));
        }

        let tool_freq = if !state.dashboard.tool_frequency.is_empty() {
            state.dashboard.tool_frequency.clone()
        } else {
            compute_tool_frequency(&state.recent_tools)
        };

        if !tool_freq.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Tool repertoire:",
                self.theme.bold_accent_style(),
            )));
            let max_count = tool_freq.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (name, count) in &tool_freq {
                let bar_len = ((*count as f64 / max_count as f64) * 8.0).ceil() as usize;
                let bar: String = "\u{2593}".repeat(bar_len.max(1));
                lines.push(Line::from(vec![
                    Span::styled(format!("   {name:<14} "), self.theme.dim_style()),
                    Span::styled(bar, self.theme.accent_style()),
                    Span::styled(format!(" {count}"), self.theme.dim_style()),
                ]));
            }
        }

        if !state.activated_skills.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" Skills activated: ", self.theme.dim_style()),
                Span::styled(
                    format!("{} this session", state.activated_skills.len()),
                    self.theme.success_style(),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("   ", self.theme.dim_style()),
                Span::styled(
                    state.activated_skills.join(", "),
                    self.theme.success_style(),
                ),
            ]));
        }

        lines
    }

    fn expanded_missions_lines(&self, state: &AppState, _inner: Rect) -> Vec<Line<'static>> {
        let routines = if !state.dashboard.routines.is_empty() {
            &state.dashboard.routines
        } else {
            &state.routines
        };

        let mut lines: Vec<Line> = Vec::new();

        if routines.is_empty() {
            lines.push(Line::from(Span::styled(
                " No missions configured",
                self.theme.dim_style(),
            )));
        } else {
            for r in routines {
                let icon = if r.enabled { "\u{25B6}" } else { "\u{23F8}" };
                let icon_style = if r.enabled {
                    self.theme.success_style()
                } else {
                    self.theme.dim_style()
                };
                let mut spans = vec![
                    Span::styled(format!(" {icon} "), icon_style),
                    Span::styled(format!("{:<20}", r.name), self.theme.accent_style()),
                    Span::styled(format!(" {:<12}", r.trigger_type), self.theme.dim_style()),
                ];
                if let Some(ref next) = r.next_fire {
                    spans.push(Span::styled(
                        format!(" next: {next}"),
                        self.theme.dim_style(),
                    ));
                }
                if let Some(ref last) = r.last_run {
                    spans.push(Span::styled(
                        format!(" last: {last}"),
                        self.theme.dim_style(),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }

        lines
    }

    /// Compute the layout rect for each dashboard panel from the main area.
    /// Returns panels in order matching `DashboardPanel` enum variants.
    pub fn panel_areas(&self, area: Rect) -> Vec<(DashboardPanel, Rect)> {
        if area.height < 6 || area.width < 30 {
            return Vec::new();
        }

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // title
                Constraint::Length(1), // summary bar
                Constraint::Length(3), // context window
                Constraint::Length(8), // token usage | top tools
                Constraint::Length(6), // system | workspace
                Constraint::Length(6), // jobs | engine threads
                Constraint::Length(7), // skills | learnings
                Constraint::Length(5), // self-learning (full width)
                Constraint::Min(3),    // missions
            ])
            .split(area);

        let cols_upper = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[3]);

        let cols_mid = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[4]);

        let cols_lower = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[5]);

        let cols_neural = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[6]);

        vec![
            (DashboardPanel::TokenUsage, cols_upper[0]),
            (DashboardPanel::TopTools, cols_upper[1]),
            (DashboardPanel::System, cols_mid[0]),
            (DashboardPanel::Workspace, cols_mid[1]),
            (DashboardPanel::Jobs, cols_lower[0]),
            (DashboardPanel::Threads, cols_lower[1]),
            (DashboardPanel::Skills, cols_neural[0]),
            (DashboardPanel::Learnings, cols_neural[1]),
            (DashboardPanel::SelfLearning, sections[7]),
            (DashboardPanel::Missions, sections[8]),
        ]
    }
}

/// Join names with spaces, truncating to fit within `max_width`.
fn join_truncated(names: &[&str], max_width: usize) -> String {
    let mut result = String::new();
    for (i, name) in names.iter().enumerate() {
        let sep = if i > 0 { "  " } else { "" };
        if result.len() + sep.len() + name.len() > max_width {
            if result.is_empty() {
                // At least show one truncated name
                result = truncate(name, max_width);
            }
            break;
        }
        result.push_str(sep);
        result.push_str(name);
    }
    result
}

/// Count tool usage from recent tools, sorted descending by count then by name.
/// Tool names are stripped to their base name (e.g. `http(https://…)` → `http`)
/// so that calls with different arguments aggregate together.
fn compute_tool_frequency(tools: &[ToolActivity]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for tool in tools {
        let base = tool
            .name
            .find('(')
            .map_or_else(|| tool.name.as_str(), |i| &tool.name[..i]);
        *counts.entry(base.to_string()).or_insert(0) += 1;
    }
    let mut freq: Vec<(String, usize)> = counts.into_iter().collect();
    // Stable sort: descending by count, ascending by name for ties
    freq.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    freq
}

/// Render a sparkline string from data points using block characters.
fn render_sparkline(data: &[u64]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let max = *data.iter().max().unwrap_or(&1);
    let max = max.max(1);
    data.iter()
        .map(|&v| {
            let idx = ((v as f64 / max as f64) * (SPARK_CHARS.len() - 1) as f64).round() as usize;
            SPARK_CHARS[idx.min(SPARK_CHARS.len() - 1)]
        })
        .collect()
}

/// Capacity bar with fill characters: `[▓▓▓▓▓▓░░░░░░░░░░░░░░]`
fn capacity_bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio * width as f64).round() as usize).min(width);
    let empty = width.saturating_sub(filled);
    let mut bar = String::with_capacity(width + 2);
    bar.push('[');
    for _ in 0..filled {
        bar.push('▓');
    }
    for _ in 0..empty {
        bar.push('░');
    }
    bar.push(']');
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_empty_data() {
        assert_eq!(render_sparkline(&[]), "");
    }

    #[test]
    fn sparkline_single_value() {
        let result = render_sparkline(&[100]);
        assert_eq!(result.chars().count(), 1);
        assert_eq!(result, "█");
    }

    #[test]
    fn sparkline_ascending() {
        let result = render_sparkline(&[0, 25, 50, 75, 100]);
        assert_eq!(result.chars().count(), 5);
        let chars: Vec<char> = result.chars().collect();
        assert_eq!(chars[0], '▁');
        assert_eq!(chars[4], '█');
    }

    #[test]
    fn capacity_bar_empty() {
        let bar = capacity_bar(0.0, 10);
        assert_eq!(bar, "[░░░░░░░░░░]");
    }

    #[test]
    fn capacity_bar_full() {
        let bar = capacity_bar(1.0, 10);
        assert_eq!(bar, "[▓▓▓▓▓▓▓▓▓▓]");
    }

    #[test]
    fn capacity_bar_half() {
        let bar = capacity_bar(0.5, 10);
        assert_eq!(bar.chars().count(), 12);
        assert!(bar.starts_with("[▓▓▓▓▓"));
        assert!(bar.ends_with("░░░░░]"));
    }

    #[test]
    fn tool_frequency_stable_sort() {
        let tools = vec![
            ToolActivity {
                call_id: None,
                name: "beta".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
            ToolActivity {
                call_id: None,
                name: "alpha".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
            ToolActivity {
                call_id: None,
                name: "beta".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
        ];
        let freq = compute_tool_frequency(&tools);
        assert_eq!(freq[0], ("beta".to_string(), 2));
        assert_eq!(freq[1], ("alpha".to_string(), 1));
    }

    #[test]
    fn tool_frequency_strips_parameters() {
        let tools = vec![
            ToolActivity {
                call_id: None,
                name: "http(https://api.github.com/repos/foo/bar)".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
            ToolActivity {
                call_id: None,
                name: "http(https://example.com/other)".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
            ToolActivity {
                call_id: None,
                name: "list_dir(.)".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
            ToolActivity {
                call_id: None,
                name: "__codeact__".to_string(),
                started_at: chrono::Utc::now(),
                duration_ms: None,
                status: super::super::ToolStatus::Success,
                detail: None,
                result_preview: None,
            },
        ];
        let freq = compute_tool_frequency(&tools);
        // http calls aggregate into one entry
        assert_eq!(freq[0], ("http".to_string(), 2));
        // names without parens are unchanged
        assert!(freq.iter().any(|(n, c)| n == "__codeact__" && *c == 1));
        assert!(freq.iter().any(|(n, c)| n == "list_dir" && *c == 1));
    }

    fn make_widget() -> DashboardWidget {
        DashboardWidget::new(Theme::default())
    }

    #[test]
    fn skills_panel_renders_without_panic() {
        let widget = make_widget();
        let state = AppState::default();
        let area = Rect::new(0, 0, 40, 7);
        let mut buf = Buffer::empty(area);
        widget.render_skills_panel(area, &mut buf, &state);
    }

    #[test]
    fn learnings_panel_renders_without_panic() {
        let widget = make_widget();
        let state = AppState::default();
        let area = Rect::new(0, 0, 40, 7);
        let mut buf = Buffer::empty(area);
        widget.render_learnings_panel(area, &mut buf, &state);
    }

    #[test]
    fn self_learning_panel_renders_without_panic() {
        let widget = make_widget();
        let state = AppState::default();
        let area = Rect::new(0, 0, 60, 5);
        let mut buf = Buffer::empty(area);
        widget.render_self_learning_panel(area, &mut buf, &state);
    }

    #[test]
    fn self_learning_computes_from_messages() {
        let widget = make_widget();
        let state = AppState {
            messages: vec![
                super::super::ChatMessage {
                    role: super::super::MessageRole::User,
                    content: "hello".to_string(),
                    timestamp: chrono::Utc::now(),
                    cost_summary: None,
                },
                super::super::ChatMessage {
                    role: super::super::MessageRole::Assistant,
                    content: "hi there".to_string(),
                    timestamp: chrono::Utc::now(),
                    cost_summary: Some(super::super::TurnCostSummary {
                        input_tokens: 1000,
                        output_tokens: 500,
                        cost_usd: "$0.01".to_string(),
                    }),
                },
                super::super::ChatMessage {
                    role: super::super::MessageRole::User,
                    content: "tell me more".to_string(),
                    timestamp: chrono::Utc::now(),
                    cost_summary: None,
                },
                super::super::ChatMessage {
                    role: super::super::MessageRole::Assistant,
                    content: "sure thing".to_string(),
                    timestamp: chrono::Utc::now(),
                    cost_summary: Some(super::super::TurnCostSummary {
                        input_tokens: 2000,
                        output_tokens: 800,
                        cost_usd: "$0.02".to_string(),
                    }),
                },
            ],
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 80, 5);
        let mut buf = Buffer::empty(area);
        widget.render_self_learning_panel(area, &mut buf, &state);
        // Should not panic and should contain sparkline data
    }

    #[test]
    fn join_truncated_fits() {
        let names = vec!["alpha", "beta", "gamma"];
        let result = join_truncated(&names, 30);
        assert_eq!(result, "alpha  beta  gamma");
    }

    #[test]
    fn join_truncated_clips() {
        let names = vec!["alpha", "beta", "gamma", "delta"];
        let result = join_truncated(&names, 15);
        assert_eq!(result, "alpha  beta");
    }

    #[test]
    fn dashboard_panel_modal_opens_on_action() {
        let mut state = AppState::default();
        assert!(state.expanded_dashboard_panel.is_none());
        state.expanded_dashboard_panel = Some(super::super::DashboardPanelModal {
            panel: super::super::DashboardPanel::Skills,
            scroll: 0,
        });
        assert!(state.expanded_dashboard_panel.is_some());
        assert_eq!(
            state.expanded_dashboard_panel.as_ref().unwrap().panel,
            super::super::DashboardPanel::Skills
        );
    }

    #[test]
    fn dashboard_panel_close_clears_modal() {
        let mut state = AppState::default();
        state.expanded_dashboard_panel = Some(super::super::DashboardPanelModal {
            panel: super::super::DashboardPanel::TopTools,
            scroll: 5,
        });
        assert!(state.expanded_dashboard_panel.is_some());
        state.expanded_dashboard_panel = None;
        assert!(state.expanded_dashboard_panel.is_none());
    }

    #[test]
    fn expanded_skills_renders_without_panic() {
        let widget = make_widget();
        let mut state = AppState::default();
        state.welcome_skills = vec![
            super::super::SkillCategory {
                name: "custom".to_string(),
                skills: vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
            },
            super::super::SkillCategory {
                name: "memory".to_string(),
                skills: vec!["recall".to_string()],
            },
        ];
        state.expanded_dashboard_panel = Some(super::super::DashboardPanelModal {
            panel: super::super::DashboardPanel::Skills,
            scroll: 0,
        });
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        widget.render_expanded_panel(area, &mut buf, &state);
    }

    #[test]
    fn expanded_tools_renders_without_panic() {
        let widget = make_widget();
        let mut state = AppState::default();
        state.expanded_dashboard_panel = Some(super::super::DashboardPanelModal {
            panel: super::super::DashboardPanel::TopTools,
            scroll: 0,
        });
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        widget.render_expanded_panel(area, &mut buf, &state);
    }

    #[test]
    fn dashboard_panel_scroll_saturates() {
        let mut modal = super::super::DashboardPanelModal {
            panel: super::super::DashboardPanel::Skills,
            scroll: 0,
        };
        // Scroll down (subtract) saturates at 0
        modal.scroll = modal.scroll.saturating_sub(3);
        assert_eq!(modal.scroll, 0);

        // Scroll up adds
        modal.scroll = modal.scroll.saturating_add(3);
        assert_eq!(modal.scroll, 3);

        modal.scroll = modal.scroll.saturating_add(3);
        assert_eq!(modal.scroll, 6);

        modal.scroll = modal.scroll.saturating_sub(3);
        assert_eq!(modal.scroll, 3);
    }

    #[test]
    fn panel_areas_returns_all_panels() {
        let widget = make_widget();
        let area = Rect::new(0, 0, 80, 50);
        let areas = widget.panel_areas(area);
        assert_eq!(areas.len(), 10);
        // Verify all panels are present
        let panels: Vec<super::super::DashboardPanel> = areas.iter().map(|(p, _)| *p).collect();
        assert!(panels.contains(&super::super::DashboardPanel::TokenUsage));
        assert!(panels.contains(&super::super::DashboardPanel::TopTools));
        assert!(panels.contains(&super::super::DashboardPanel::System));
        assert!(panels.contains(&super::super::DashboardPanel::Workspace));
        assert!(panels.contains(&super::super::DashboardPanel::Jobs));
        assert!(panels.contains(&super::super::DashboardPanel::Threads));
        assert!(panels.contains(&super::super::DashboardPanel::Skills));
        assert!(panels.contains(&super::super::DashboardPanel::Learnings));
        assert!(panels.contains(&super::super::DashboardPanel::SelfLearning));
        assert!(panels.contains(&super::super::DashboardPanel::Missions));
    }

    #[test]
    fn panel_areas_empty_for_tiny_area() {
        let widget = make_widget();
        let area = Rect::new(0, 0, 10, 3);
        let areas = widget.panel_areas(area);
        assert!(areas.is_empty());
    }
}
