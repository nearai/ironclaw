//! Inline plan checklist rendering for the conversation view.
//!
//! Renders a Claude Code-style plan block with step icons, progress bar,
//! status badge, and animated in-progress spinner.

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

use super::{PlanState, PlanStatus, PlanStepStatus};

/// Animated spinner characters for in-progress steps.
const SPINNER_CHARS: &[&str] = &["\u{25D0}", "\u{25D1}", "\u{25D2}", "\u{25D3}"]; // ◐◑◒◓

/// Render the plan checklist block inline in the conversation.
pub fn render_plan_block(
    plan: &PlanState,
    usable_width: usize,
    tick_count: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(render_plan_header(plan, usable_width, theme));
    lines.push(render_progress_line(plan, usable_width, theme));
    lines.push(Line::from(""));

    for step in &plan.steps {
        lines.push(render_step(step, usable_width, tick_count, theme));
        if let Some(ref result) = step.result {
            lines.push(render_step_result(result, usable_width, theme));
        }
    }

    lines.push(Line::from(""));
    lines
}

/// Top border line: `  ──── plan ──────────────── STATUS`
fn render_plan_header(plan: &PlanState, usable_width: usize, theme: &Theme) -> Line<'static> {
    let label = " plan ";
    let badge = format!(" {} ", plan.status.label());

    let badge_style = match plan.status {
        PlanStatus::Draft | PlanStatus::Approved => theme.warning_style(),
        PlanStatus::Executing => theme.accent_style(),
        PlanStatus::Completed => theme.success_style(),
        PlanStatus::Failed => theme.error_style(),
    };

    // Build the rule line: "  ──── plan ─────────────── STATUS"
    let prefix_dashes = 4;
    let suffix_dashes = usable_width
        .saturating_sub(2) // leading spaces
        .saturating_sub(prefix_dashes)
        .saturating_sub(label.len())
        .saturating_sub(badge.len())
        .saturating_sub(1); // trailing space

    let prefix_rule: String = "\u{2500}".repeat(prefix_dashes);
    let suffix_rule: String = "\u{2500}".repeat(suffix_dashes);

    Line::from(vec![
        Span::styled("  ", theme.dim_style()),
        Span::styled(prefix_rule, theme.dim_style()),
        Span::styled(
            label.to_string(),
            theme.dim_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(suffix_rule, theme.dim_style()),
        Span::styled(badge, badge_style.add_modifier(Modifier::BOLD)),
    ])
}

/// Progress summary: `  Title                 3/7 completed  [████        ]`
fn render_progress_line(plan: &PlanState, usable_width: usize, theme: &Theme) -> Line<'static> {
    let completed = plan.completed_count();
    let total = plan.steps.len();
    let progress_text = format!("{completed}/{total} completed");

    // Mini progress bar
    let bar_width = 16.min(usable_width.saturating_sub(8));
    let bar = if total > 0 {
        let filled = (completed * bar_width) / total;
        let empty = bar_width.saturating_sub(filled);
        format!("[{}{}]", "\u{2588}".repeat(filled), " ".repeat(empty),)
    } else {
        format!("[{}]", " ".repeat(bar_width))
    };

    // Truncate title if needed
    let title_max = usable_width
        .saturating_sub(4) // "  " prefix + padding
        .saturating_sub(progress_text.len())
        .saturating_sub(bar.len())
        .saturating_sub(4); // spacing

    let title = if plan.title.len() > title_max {
        format!("{}...", &plan.title[..title_max.saturating_sub(3)])
    } else {
        plan.title.clone()
    };

    Line::from(vec![
        Span::styled("  ", theme.dim_style()),
        Span::styled(title, theme.accent_style().add_modifier(Modifier::BOLD)),
        Span::styled("  ", theme.dim_style()),
        Span::styled(progress_text, theme.dim_style()),
        Span::styled("  ", theme.dim_style()),
        Span::styled(bar, theme.accent_style()),
    ])
}

/// A single step line: `   1 ● Title text`
fn render_step(
    step: &super::PlanStep,
    _usable_width: usize,
    tick_count: usize,
    theme: &Theme,
) -> Line<'static> {
    let number = format!("   {:>2} ", step.index + 1);

    let icon = match step.status {
        PlanStepStatus::InProgress => {
            let idx = (tick_count / 8) % SPINNER_CHARS.len();
            SPINNER_CHARS[idx].to_string()
        }
        other => other.icon().to_string(),
    };

    let icon_style = match step.status {
        PlanStepStatus::Pending => theme.dim_style(),
        PlanStepStatus::InProgress => theme.accent_style(),
        PlanStepStatus::Completed => theme.success_style(),
        PlanStepStatus::Failed => theme.error_style(),
    };

    let title_style = match step.status {
        PlanStepStatus::Pending => theme.dim_style(),
        PlanStepStatus::Completed => theme.success_style(),
        _ => theme.accent_style(),
    };

    Line::from(vec![
        Span::styled(number, theme.dim_style()),
        Span::styled(format!("{icon} "), icon_style),
        Span::styled(step.title.clone(), title_style),
    ])
}

/// Step result line: `        → result text`
fn render_step_result(result: &str, usable_width: usize, theme: &Theme) -> Line<'static> {
    let prefix = "        \u{2192} ";
    let max_len = usable_width.saturating_sub(prefix.len());
    let display = if result.len() > max_len {
        format!("{}...", &result[..max_len.saturating_sub(3)])
    } else {
        result.to_string()
    };

    Line::from(vec![
        Span::styled("        \u{2192} ".to_string(), theme.dim_style()),
        Span::styled(display, theme.dim_style()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::widgets::{PlanState, PlanStatus, PlanStep, PlanStepStatus};

    fn sample_plan() -> PlanState {
        PlanState {
            plan_id: "plan-1".to_string(),
            title: "Set up CI/CD pipeline".to_string(),
            status: PlanStatus::Executing,
            steps: vec![
                PlanStep {
                    index: 0,
                    title: "Create workflow".to_string(),
                    status: PlanStepStatus::Completed,
                    result: None,
                },
                PlanStep {
                    index: 1,
                    title: "Configure build".to_string(),
                    status: PlanStepStatus::Completed,
                    result: None,
                },
                PlanStep {
                    index: 2,
                    title: "Add test runner".to_string(),
                    status: PlanStepStatus::Completed,
                    result: Some("All 47 tests passing".to_string()),
                },
                PlanStep {
                    index: 3,
                    title: "Set up deployment".to_string(),
                    status: PlanStepStatus::InProgress,
                    result: None,
                },
                PlanStep {
                    index: 4,
                    title: "Configure staging".to_string(),
                    status: PlanStepStatus::Pending,
                    result: None,
                },
            ],
            mission_id: None,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn plan_status_from_str() {
        assert_eq!(PlanStatus::parse_status("draft"), PlanStatus::Draft);
        assert_eq!(PlanStatus::parse_status("DRAFT"), PlanStatus::Draft);
        assert_eq!(PlanStatus::parse_status("approved"), PlanStatus::Approved);
        assert_eq!(PlanStatus::parse_status("executing"), PlanStatus::Executing);
        assert_eq!(PlanStatus::parse_status("running"), PlanStatus::Executing);
        assert_eq!(
            PlanStatus::parse_status("in_progress"),
            PlanStatus::Executing
        );
        assert_eq!(PlanStatus::parse_status("completed"), PlanStatus::Completed);
        assert_eq!(PlanStatus::parse_status("done"), PlanStatus::Completed);
        assert_eq!(PlanStatus::parse_status("failed"), PlanStatus::Failed);
        assert_eq!(PlanStatus::parse_status("error"), PlanStatus::Failed);
        assert_eq!(PlanStatus::parse_status("unknown"), PlanStatus::Draft);
    }

    #[test]
    fn step_status_icon() {
        assert_eq!(PlanStepStatus::Pending.icon(), "\u{25CB}"); // ○
        assert_eq!(PlanStepStatus::InProgress.icon(), "\u{25D0}"); // ◐
        assert_eq!(PlanStepStatus::Completed.icon(), "\u{25CF}"); // ●
        assert_eq!(PlanStepStatus::Failed.icon(), "\u{2715}"); // ✕
    }

    #[test]
    fn step_status_from_str() {
        assert_eq!(
            PlanStepStatus::parse_status("pending"),
            PlanStepStatus::Pending
        );
        assert_eq!(
            PlanStepStatus::parse_status("in_progress"),
            PlanStepStatus::InProgress
        );
        assert_eq!(
            PlanStepStatus::parse_status("running"),
            PlanStepStatus::InProgress
        );
        assert_eq!(
            PlanStepStatus::parse_status("completed"),
            PlanStepStatus::Completed
        );
        assert_eq!(
            PlanStepStatus::parse_status("done"),
            PlanStepStatus::Completed
        );
        assert_eq!(
            PlanStepStatus::parse_status("failed"),
            PlanStepStatus::Failed
        );
        assert_eq!(
            PlanStepStatus::parse_status("error"),
            PlanStepStatus::Failed
        );
        assert_eq!(
            PlanStepStatus::parse_status("anything"),
            PlanStepStatus::Pending
        );
    }

    #[test]
    fn render_plan_block_basic() {
        let plan = sample_plan();
        let theme = Theme::default();
        let lines = render_plan_block(&plan, 80, 0, &theme);
        // Should have: header + progress + blank + 5 steps + 1 result + blank = 10
        assert!(!lines.is_empty());
        assert!(lines.len() >= 8);
    }

    #[test]
    fn render_plan_block_empty_steps() {
        let plan = PlanState {
            plan_id: "plan-2".to_string(),
            title: "Empty plan".to_string(),
            status: PlanStatus::Draft,
            steps: vec![],
            mission_id: None,
            updated_at: chrono::Utc::now(),
        };
        let theme = Theme::default();
        let lines = render_plan_block(&plan, 80, 0, &theme);
        // header + progress + blank + blank = 4
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn progress_count() {
        let plan = sample_plan();
        assert_eq!(plan.completed_count(), 3);
    }

    #[test]
    fn terminal_state() {
        assert!(PlanStatus::Completed.is_terminal());
        assert!(PlanStatus::Failed.is_terminal());
        assert!(!PlanStatus::Draft.is_terminal());
        assert!(!PlanStatus::Approved.is_terminal());
        assert!(!PlanStatus::Executing.is_terminal());
    }
}
