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

        let total_tokens = state.total_input_tokens + state.total_output_tokens;
        let tokens_used_str = format_tokens(total_tokens);
        let context_max_str = format_tokens(state.context_window);

        let ratio = if state.context_window > 0 {
            (total_tokens as f64 / state.context_window as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let pct = (ratio * 100.0).round() as u64;

        // Session duration
        let elapsed = chrono::Utc::now()
            .signed_duration_since(state.session_start)
            .num_seconds()
            .unsigned_abs();
        let duration_str = format_duration(elapsed);

        let sep = Span::styled(" \u{2502} ", self.theme.dim_style());

        let tab_label = match state.active_tab {
            ActiveTab::Conversation => "[Chat]",
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
            Span::styled(
                format!("v{}", state.version),
                self.theme.dim_style(),
            ),
            sep.clone(),
            Span::styled(
                format!("{tokens_used_str}/{context_max_str}"),
                self.theme.dim_style(),
            ),
            sep.clone(),
            Span::styled(bar_str, bar_style),
            Span::styled(format!(" {pct}%"), self.theme.dim_style()),
        ];

        if state.total_cost_usd != "$0.00" {
            left_spans.push(sep.clone());
            left_spans.push(Span::styled(
                state.total_cost_usd.clone(),
                self.theme.dim_style(),
            ));
        }

        left_spans.push(sep);
        left_spans.push(Span::styled(duration_str, self.theme.dim_style()));

        let right_text = "^L logs  ^B sidebar  ^C quit";
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
