//! Optional chat sidebar showing current work context and plan steps.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::{format_tokens, truncate};
use crate::theme::Theme;

use super::{AppState, MessageRole, PlanState, PlanStepStatus, ToolStatus, TuiWidget};

pub struct WorkSidebarWidget {
    theme: Theme,
}

#[derive(Debug, Clone)]
struct SidebarPlan {
    title: String,
    steps: Vec<SidebarPlanStep>,
}

#[derive(Debug, Clone)]
struct SidebarPlanStep {
    title: String,
    status: PlanStepStatus,
}

impl WorkSidebarWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    fn section_header(&self, title: &str) -> Line<'static> {
        Line::from(Span::styled(
            format!("  {title}"),
            self.theme.accent_style().add_modifier(Modifier::BOLD),
        ))
    }

    fn bullet(&self, text: String, style: Style) -> Line<'static> {
        Line::from(vec![
            Span::styled("  \u{2022} ".to_string(), self.theme.dim_style()),
            Span::styled(text, style),
        ])
    }

    fn build_lines(&self, state: &AppState, width: usize) -> Vec<Line<'static>> {
        let content_width = width.saturating_sub(4).max(8);
        let mut lines = Vec::new();

        let message_plan = Self::latest_message_plan(state);

        lines.extend(self.title_lines(state, message_plan.as_ref(), content_width));

        if let Some(ref plan) = state.plan_state
            && !plan.steps.is_empty()
        {
            lines.push(Line::from(""));
            lines.extend(self.plan_lines(plan, content_width));
        } else if let Some(ref plan) = message_plan
            && !plan.steps.is_empty()
        {
            lines.push(Line::from(""));
            lines.extend(self.sidebar_plan_lines(plan, content_width));
        }

        lines.push(Line::from(""));
        lines.extend(self.context_lines(state));

        let tool_lines = self.tool_lines(state, content_width);
        if !tool_lines.is_empty() {
            lines.push(Line::from(""));
            lines.extend(tool_lines);
        }

        lines
    }

    fn title_lines(
        &self,
        state: &AppState,
        message_plan: Option<&SidebarPlan>,
        width: usize,
    ) -> Vec<Line<'static>> {
        let title = state
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
            .or_else(|| state.plan_state.as_ref().map(|plan| plan.title.as_str()))
            .or_else(|| message_plan.map(|plan| plan.title.as_str()))
            .unwrap_or("No active turn");

        let mut lines = vec![Line::from(Span::styled(
            format!("  {}", truncate(title, width)),
            self.theme.bold_accent_style(),
        ))];

        if let Some(ref thread_id) = state.current_thread_id {
            lines.push(self.bullet(
                format!("thread {}", truncate(thread_id, width.saturating_sub(7))),
                self.theme.dim_style(),
            ));
        }

        if !state.status_text.is_empty() {
            lines.push(self.bullet(
                truncate(&state.status_text, width),
                self.theme.warning_style(),
            ));
        } else if state.is_streaming {
            lines.push(self.bullet("streaming".to_string(), self.theme.accent_style()));
        }

        lines
    }

    fn plan_lines(&self, plan: &PlanState, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(self.section_header("Plan"));
        lines.push(self.bullet(truncate(&plan.title, width), self.theme.dim_style()));

        let completed = plan.completed_count();
        let total = plan.steps.len();
        lines.push(self.bullet(
            format!("{completed}/{total} completed"),
            self.theme.dim_style(),
        ));

        for step in &plan.steps {
            let (icon, style) = match step.status {
                PlanStepStatus::Pending => ("[ ]", self.theme.dim_style()),
                PlanStepStatus::InProgress => ("[o]", self.theme.warning_style()),
                PlanStepStatus::Completed => ("[x]", self.theme.success_style()),
                PlanStepStatus::Failed => ("[!]", self.theme.error_style()),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {icon} "), style),
                Span::styled(truncate(&step.title, width.saturating_sub(6)), style),
            ]));
        }

        lines
    }

    fn sidebar_plan_lines(&self, plan: &SidebarPlan, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(self.section_header("Plan"));
        lines.push(self.bullet(truncate(&plan.title, width), self.theme.dim_style()));

        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        let total = plan.steps.len();
        lines.push(self.bullet(
            format!("{completed}/{total} completed"),
            self.theme.dim_style(),
        ));

        for step in &plan.steps {
            let (icon, style) = match step.status {
                PlanStepStatus::Pending => ("[ ]", self.theme.dim_style()),
                PlanStepStatus::InProgress => ("[o]", self.theme.warning_style()),
                PlanStepStatus::Completed => ("[x]", self.theme.success_style()),
                PlanStepStatus::Failed => ("[!]", self.theme.error_style()),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {icon} "), style),
                Span::styled(truncate(&step.title, width.saturating_sub(6)), style),
            ]));
        }

        lines
    }

    fn context_lines(&self, state: &AppState) -> Vec<Line<'static>> {
        let (used_tokens, max_tokens, pct) = if let Some(ref pressure) = state.context_pressure {
            (
                pressure.used_tokens,
                pressure.max_tokens,
                pressure.percentage as u64,
            )
        } else {
            (0, state.context_window, 0)
        };

        vec![
            self.section_header("Context"),
            self.bullet(
                format!(
                    "{}/{} tokens",
                    format_tokens(used_tokens),
                    format_tokens(max_tokens)
                ),
                self.theme.dim_style(),
            ),
            self.bullet(format!("{pct}% used"), self.context_style(pct)),
            self.bullet(
                format!("{} spent", state.total_cost_usd),
                self.theme.dim_style(),
            ),
        ]
    }

    fn context_style(&self, pct: u64) -> Style {
        if pct >= 90 {
            self.theme.error_style()
        } else if pct >= 70 {
            self.theme.warning_style()
        } else {
            self.theme.dim_style()
        }
    }

    fn tool_lines(&self, state: &AppState, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if state.active_tools.is_empty() && state.recent_tools.is_empty() {
            return lines;
        }

        lines.push(self.section_header("Tools"));

        for tool in state.active_tools.iter().take(5) {
            let detail = tool
                .detail
                .as_deref()
                .map(|detail| format!(" {}", truncate(detail, width.saturating_sub(14))))
                .unwrap_or_default();
            lines.push(self.bullet(
                truncate(&format!("{} running{detail}", tool.name), width),
                self.theme.warning_style(),
            ));
        }

        let remaining = 5usize.saturating_sub(state.active_tools.len().min(5));
        if remaining > 0 {
            for tool in state.recent_tools.iter().rev().take(remaining) {
                let style = match tool.status {
                    ToolStatus::Running => self.theme.warning_style(),
                    ToolStatus::Success => self.theme.success_style(),
                    ToolStatus::Failed => self.theme.error_style(),
                };
                let status = match tool.status {
                    ToolStatus::Running => "running",
                    ToolStatus::Success => "done",
                    ToolStatus::Failed => "failed",
                };
                lines.push(self.bullet(truncate(&format!("{} {status}", tool.name), width), style));
            }
        }

        lines
    }

    fn latest_message_plan(state: &AppState) -> Option<SidebarPlan> {
        state
            .messages
            .iter()
            .rev()
            .filter(|message| message.role != MessageRole::User)
            .find_map(|message| Self::parse_message_plan(&message.content))
    }

    fn parse_message_plan(content: &str) -> Option<SidebarPlan> {
        let lines: Vec<&str> = content.lines().collect();
        let (title, start_idx) = Self::find_plan_title_and_step_start(&lines)?;

        let steps = lines[start_idx..]
            .iter()
            .filter_map(|line| Self::parse_ordered_step(line))
            .collect::<Vec<_>>();

        (!steps.is_empty()).then_some(SidebarPlan { title, steps })
    }

    fn find_plan_title_and_step_start(lines: &[&str]) -> Option<(String, usize)> {
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if let Some(title) = trimmed
                .strip_prefix("Plan ready:")
                .or_else(|| trimmed.strip_prefix("Plan approved:"))
            {
                return Some((Self::normalize_plan_title(title), idx + 1));
            }

            if trimmed == "Plan checklist:" {
                return Some((
                    Self::saved_plan_title(lines).unwrap_or_else(|| "Plan".to_string()),
                    idx + 1,
                ));
            }
        }

        None
    }

    fn saved_plan_title(lines: &[&str]) -> Option<String> {
        lines.iter().enumerate().find_map(|(idx, line)| {
            if line.trim() != "Saved plan:" {
                return None;
            }
            lines[idx + 1..].iter().find_map(|candidate| {
                let trimmed = candidate.trim();
                trimmed
                    .strip_prefix("- ")
                    .or_else(|| trimmed.strip_prefix("* "))
                    .map(Self::normalize_plan_title)
            })
        })
    }

    fn normalize_plan_title(title: &str) -> String {
        let title = title.trim();
        if title.is_empty() {
            "Plan".to_string()
        } else {
            title.to_string()
        }
    }

    fn parse_ordered_step(line: &str) -> Option<SidebarPlanStep> {
        let trimmed = line.trim_start();
        let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if digit_count == 0 {
            return None;
        }

        let rest = trimmed[digit_count..].strip_prefix('.')?.trim_start();
        let (status, title) = if let Some(title) = rest.strip_prefix("[x]") {
            (PlanStepStatus::Completed, title)
        } else if let Some(title) = rest.strip_prefix("[X]") {
            (PlanStepStatus::Completed, title)
        } else if let Some(title) = rest.strip_prefix("[o]") {
            (PlanStepStatus::InProgress, title)
        } else if let Some(title) = rest.strip_prefix("[O]") {
            (PlanStepStatus::InProgress, title)
        } else if let Some(title) = rest.strip_prefix("[!]") {
            (PlanStepStatus::Failed, title)
        } else if let Some(title) = rest.strip_prefix("[ ]") {
            (PlanStepStatus::Pending, title)
        } else {
            (PlanStepStatus::Pending, rest)
        };

        let title = title.trim();
        let (title, status) = if let Some(title) = title.strip_suffix(" - completed") {
            (title.trim_end(), PlanStepStatus::Completed)
        } else if let Some(title) = title.strip_suffix(" - done") {
            (title.trim_end(), PlanStepStatus::Completed)
        } else if let Some(title) = title.strip_suffix(" - failed") {
            (title.trim_end(), PlanStepStatus::Failed)
        } else if let Some(title) = title.strip_suffix(" - in progress") {
            (title.trim_end(), PlanStepStatus::InProgress)
        } else {
            (title, status)
        };
        if title.is_empty() {
            return None;
        }

        Some(SidebarPlanStep {
            title: title.to_string(),
            status,
        })
    }
}

impl TuiWidget for WorkSidebarWidget {
    fn id(&self) -> &str {
        "work_sidebar"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 24 {
            return;
        }

        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(self.theme.border_style());
        let lines = self.build_lines(state, area.width as usize);
        Paragraph::new(lines).block(block).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{ChatMessage, PlanState, PlanStatus, PlanStep};

    fn buffer_text(buf: &Buffer, area: Rect) -> String {
        let mut lines = Vec::new();
        for y in area.y..area.y + area.height {
            let mut line = String::new();
            for x in area.x..area.x + area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    #[test]
    fn work_sidebar_renders_plan_on_right() {
        let widget = WorkSidebarWidget::new(Theme::dark());
        let state = AppState {
            plan_state: Some(PlanState {
                plan_id: "plan-1".to_string(),
                title: "Investigate engine bug".to_string(),
                status: PlanStatus::Executing,
                steps: vec![
                    PlanStep {
                        index: 0,
                        title: "Fetch issue details".to_string(),
                        status: PlanStepStatus::InProgress,
                        result: None,
                    },
                    PlanStep {
                        index: 1,
                        title: "Read engine paths".to_string(),
                        status: PlanStepStatus::Pending,
                        result: None,
                    },
                ],
                mission_id: None,
                updated_at: chrono::Utc::now(),
            }),
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Investigate engine bug"));
        assert!(text.contains("Plan"));
        assert!(!text.contains("Working"));
        assert!(!text.contains("Todo"));
        assert!(text.contains("Fetch issue details"));
    }

    #[test]
    fn work_sidebar_extracts_plan_from_assistant_message() {
        let widget = WorkSidebarWidget::new(Theme::dark());
        let now = chrono::Utc::now();
        let state = AppState {
            messages: vec![
                ChatMessage {
                    role: MessageRole::User,
                    content: "approve the plan".to_string(),
                    timestamp: now,
                    cost_summary: None,
                },
                ChatMessage {
                    role: MessageRole::Assistant,
                    content: "Plan approved: tui-expandable-tool-calls\n\nNext execution order:\n\n1. Audit current transcript history and tool-activity flow end-to-end\n2. Design the new TUI transcript/turn data model\n3. Design backend/web history payload changes".to_string(),
                    timestamp: now,
                    cost_summary: None,
                },
            ],
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 48, 16);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("approve the plan"));
        assert!(text.contains("Plan"));
        assert!(text.contains("tui-expandable-tool-calls"));
        assert!(text.contains("Audit current transcript"));
        assert!(text.contains("0/3 completed"));
    }

    #[test]
    fn work_sidebar_extracts_saved_plan_checklist() {
        let widget = WorkSidebarWidget::new(Theme::dark());
        let now = chrono::Utc::now();
        let state = AppState {
            messages: vec![
                ChatMessage {
                    role: MessageRole::User,
                    content: "yes do it and give me a plan to approve".to_string(),
                    timestamp: now,
                    cost_summary: None,
                },
                ChatMessage {
                    role: MessageRole::Assistant,
                    content: "Saved plan:\n\n- plans/twitter-wasm-tool.md\n\nApproval summary:\n\n- Tool name: twitter\n\nPlan checklist:\n\n1. Gather context on Ironclaw repo and existing decisions - completed\n2. Inspect existing WASM tool implementations and extension architecture - completed\n3. Define Twitter tool scope, auth model, and API surface - completed\n4. Design V1 contract, manifest, and runtime capabilities - completed\n5. Plan implementation of the Twitter WASM tool package - completed\n6. Plan host integration, activation, and secret handling - completed\n7. Plan tests, validation, and rollout - completed".to_string(),
                    timestamp: now,
                    cost_summary: None,
                },
            ],
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 54, 18);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("yes do it and give me a plan"));
        assert!(text.contains("plans/twitter-wasm-tool.md"));
        assert!(text.contains("7/7 completed"));
        assert!(text.contains("Gather context on Ironclaw"));
        assert!(text.contains("[x]"));
    }
}
