//! Status bar widget: model, tokens, context bar, cost, and session duration.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::{format_duration, format_tokens, truncate};
use crate::theme::Theme;

use super::{ActiveTab, AppState, PlanStatus, TuiWidget};

pub struct StatusBarWidget {
    theme: Theme,
}

impl StatusBarWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

/// Build a text-based progress bar: `[████░░░░░░░░░░]`
fn context_bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio * width as f64).round() as usize).min(width);
    let empty = width.saturating_sub(filled);
    let mut bar = String::with_capacity(width + 2);
    bar.push('[');
    for _ in 0..filled {
        bar.push('\u{2588}'); // █
    }
    for _ in 0..empty {
        bar.push(' ');
    }
    bar.push(']');
    bar
}

impl TuiWidget for StatusBarWidget {
    fn id(&self) -> &str {
        "status_bar"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::StatusBarLeft
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Use actual context pressure data from the engine when available.
        // Lifetime token spend is intentionally not used as a fallback because
        // it only grows and does not represent the active thread context.
        let (used_tokens, max_tokens, pct) = if let Some(ref cp) = state.context_pressure {
            (cp.used_tokens, cp.max_tokens, cp.percentage as u64)
        } else {
            let ctx = state.context_window;
            (0, ctx, 0)
        };
        let tokens_used_str = format_tokens(used_tokens);
        let context_max_str = format_tokens(max_tokens);
        let ratio = if max_tokens > 0 {
            (used_tokens as f64 / max_tokens as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Session duration
        let elapsed = chrono::Utc::now()
            .signed_duration_since(state.session_start)
            .num_seconds()
            .unsigned_abs();
        let duration_str = format_duration(elapsed);

        let sep = Span::styled(" \u{2502} ", self.theme.dim_style());

        let tab_label = match state.active_tab {
            ActiveTab::Conversation => "[Chat]",
            ActiveTab::Dashboard => "[Dash]",
            ActiveTab::Logs => "[Logs]",
            ActiveTab::Settings => "[Settings]",
        };

        // Bar width adapts to terminal: use ~16 chars on wide terminals, less on narrow
        let bar_width = if area.width > 100 {
            16
        } else if area.width > 60 {
            10
        } else {
            6
        };
        let bar_str = context_bar(ratio, bar_width);

        // Color the bar based on usage
        let bar_style = if pct >= 90 {
            self.theme.error_style()
        } else if pct >= 70 {
            self.theme.warning_style()
        } else {
            self.theme.accent_style()
        };

        let mut left_spans = vec![
            Span::styled(format!(" {tab_label} "), self.theme.bold_accent_style()),
            sep.clone(),
            Span::styled(state.model.to_string(), self.theme.accent_style()),
            sep.clone(),
            Span::styled(format!("v{}", state.version), self.theme.dim_style()),
        ];

        // Fleet/activity summary: active tools and threads
        let tool_count = state.active_tools.len();
        let thread_count = state.threads.len();
        if tool_count > 0 || thread_count > 0 {
            left_spans.push(sep.clone());
            let mut parts: Vec<Span> = Vec::new();
            if tool_count > 0 {
                parts.push(Span::styled(
                    format!("\u{26A1}{tool_count} tools"),
                    self.theme.accent_style(),
                ));
            }
            if tool_count > 0 && thread_count > 0 {
                parts.push(Span::styled(" \u{00B7} ", self.theme.dim_style()));
            }
            if thread_count > 0 {
                parts.push(Span::styled(
                    format!("\u{25C6}{thread_count} threads"),
                    self.theme.dim_style(),
                ));
            }
            left_spans.extend(parts);
        }

        let mut right_plan: Option<(String, Style)> = None;

        // Plan indicator. Wide terminals show the more useful title on the
        // right; narrow terminals keep a compact progress count on the left.
        if let Some(ref plan) = state.plan_state {
            let plan_style = match plan.status {
                PlanStatus::Draft | PlanStatus::Approved => self.theme.warning_style(),
                PlanStatus::Executing => self.theme.accent_style(),
                PlanStatus::Completed => self.theme.success_style(),
                PlanStatus::Failed => self.theme.error_style(),
            };
            let completed = plan.completed_count();
            let total = plan.steps.len();
            let title_width = if area.width >= 140 {
                Some(36)
            } else if area.width >= 112 {
                Some(28)
            } else if area.width >= 96 {
                Some(20)
            } else {
                None
            };
            if let Some(title_width) = title_width {
                right_plan = Some((
                    format!(
                        "\u{25A3} {} {completed}/{total}",
                        truncate(&plan.title, title_width)
                    ),
                    plan_style,
                ));
            } else {
                left_spans.push(sep.clone());
                left_spans.push(Span::styled(
                    format!("\u{25A3} Plan {completed}/{total}"),
                    plan_style,
                ));
            }
        }

        // Context pressure: tokens + visual bar
        left_spans.extend([
            sep.clone(),
            Span::styled(
                format!("{tokens_used_str}/{context_max_str}"),
                self.theme.dim_style(),
            ),
            sep.clone(),
            Span::styled(bar_str, bar_style),
            Span::styled(format!(" {pct}%"), self.theme.dim_style()),
        ]);

        // Context pressure warning when usage is high
        if let Some(ref cp) = state.context_pressure {
            if let Some(ref warning) = cp.warning {
                left_spans.push(Span::styled(
                    format!(" {warning}"),
                    if cp.percentage >= 90 {
                        self.theme.error_style()
                    } else {
                        self.theme.warning_style()
                    },
                ));
            }
        } else if pct >= 90 {
            left_spans.push(Span::styled(" CRITICAL", self.theme.error_style()));
        } else if pct >= 70 {
            left_spans.push(Span::styled(" HIGH", self.theme.warning_style()));
        }

        // Cost: show session spending, and budget if available
        if let Some(ref cg) = state.cost_guard {
            left_spans.push(sep.clone());
            if let Some(ref budget) = cg.session_budget_usd {
                // Show spent/budget with color coding
                let cost_style = if cg.limit_reached {
                    self.theme.error_style()
                } else {
                    self.theme.dim_style()
                };
                left_spans.push(Span::styled(
                    format!("{}/{budget}", cg.spent_usd),
                    cost_style,
                ));
                if cg.limit_reached {
                    left_spans.push(Span::styled(" LIMIT", self.theme.error_style()));
                }
            } else {
                left_spans.push(Span::styled(cg.spent_usd.clone(), self.theme.dim_style()));
            }
        } else if state.total_cost_usd != "$0.00" {
            left_spans.push(sep.clone());
            left_spans.push(Span::styled(
                state.total_cost_usd.clone(),
                self.theme.dim_style(),
            ));
        }

        left_spans.push(sep);
        left_spans.push(Span::styled(duration_str, self.theme.dim_style()));

        // Render left-aligned portion
        let left_line = Line::from(left_spans);
        let left_widget =
            ratatui::widgets::Paragraph::new(left_line).style(self.theme.status_style());
        left_widget.render(area, buf);

        if let Some((right_text, plan_style)) = right_plan {
            let right_width = (right_text.chars().count() + 2)
                .min(area.width.saturating_sub(1) as usize)
                .min((area.width as usize / 2).max(24)) as u16;
            if right_width > 0 && right_width < area.width {
                let right_area = Rect {
                    x: area.x + area.width - right_width,
                    y: area.y,
                    width: right_width,
                    height: area.height,
                };
                let right_line = Line::from(Span::styled(format!("{right_text} "), plan_style));
                ratatui::widgets::Paragraph::new(right_line)
                    .alignment(Alignment::Right)
                    .style(self.theme.status_style())
                    .render(right_area, buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::widgets::{PlanState, PlanStep, PlanStepStatus};

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
    fn context_bar_empty() {
        let bar = context_bar(0.0, 10);
        assert_eq!(bar, "[          ]");
    }

    #[test]
    fn context_bar_full() {
        let bar = context_bar(1.0, 10);
        assert_eq!(
            bar,
            "[\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}]"
        );
    }

    #[test]
    fn context_bar_half() {
        let bar = context_bar(0.5, 10);
        // 5 filled + 5 empty
        assert_eq!(bar.chars().count(), 12); // [ + 10 + ]
        assert!(bar.starts_with("[\u{2588}"));
        assert!(bar.ends_with(" ]"));
    }

    #[test]
    fn context_bar_clamped_over_1() {
        let bar = context_bar(1.5, 10);
        // Should be same as 1.0
        assert_eq!(bar, context_bar(1.0, 10));
    }

    #[test]
    fn renders_compact_plan_title_on_wide_status_bar() {
        let widget = StatusBarWidget::new(Theme::dark());
        let state = AppState {
            plan_state: Some(PlanState {
                plan_id: "twitter".to_string(),
                title: "Twitter WASM tool rollout".to_string(),
                status: PlanStatus::Executing,
                steps: vec![
                    PlanStep {
                        index: 0,
                        title: "Gather context".to_string(),
                        status: PlanStepStatus::Completed,
                        result: None,
                    },
                    PlanStep {
                        index: 1,
                        title: "Implement tool".to_string(),
                        status: PlanStepStatus::Pending,
                        result: None,
                    },
                ],
                mission_id: None,
                updated_at: chrono::Utc::now(),
            }),
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 140, 1);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Twitter WASM tool rollout 1/2"));
        assert!(!text.contains("^/ help"));
    }
}
