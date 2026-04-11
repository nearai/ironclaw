//! Status bar widget: model, tokens, context bar, cost, session duration, keybind hints.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::{format_duration, format_tokens};
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

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

        // Use actual context pressure data from the engine when available;
        // fall back to cumulative session tokens before the first update arrives.
        let (used_tokens, max_tokens, pct) = if let Some(ref cp) = state.context_pressure {
            (cp.used_tokens, cp.max_tokens, cp.percentage as u64)
        } else {
            let total = state.total_input_tokens + state.total_output_tokens;
            let ctx = state.context_window;
            let p = if ctx > 0 {
                ((total as f64 / ctx as f64).clamp(0.0, 1.0) * 100.0).round() as u64
            } else {
                0
            };
            (total, ctx, p)
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

        let right_text = "^L tabs  ^B dashboard  ^C quit";
        let right_span = Span::styled(format!("{right_text}  "), self.theme.dim_style());

        // Render left-aligned portion
        let left_line = Line::from(left_spans);
        let left_widget =
            ratatui::widgets::Paragraph::new(left_line).style(self.theme.status_style());
        left_widget.render(area, buf);

        // Render right-aligned keybind hints
        let right_line = Line::from(right_span);
        let right_widget = ratatui::widgets::Paragraph::new(right_line)
            .alignment(Alignment::Right)
            .style(self.theme.status_style());
        right_widget.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
