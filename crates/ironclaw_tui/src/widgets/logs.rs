//! Logs widget: scrollable log viewer with color-coded levels.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub struct LogsWidget {
    theme: Theme,
}

impl LogsWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Style for a log level string.
    fn level_style(&self, level: &str) -> Style {
        match level {
            "ERROR" => self.theme.error_style(),
            "WARN" => self.theme.warning_style(),
            "INFO" => self.theme.success_style(),
            _ => self.theme.dim_style(), // DEBUG, TRACE
        }
    }

    /// Shorten an ISO 8601 timestamp to HH:MM:SS.mmm for density.
    fn short_timestamp(ts: &str) -> &str {
        // Input: "2024-01-01T08:00:36.095Z" → want "08:00:36.095"
        // Find 'T' and take the next 12 chars
        if let Some(t_pos) = ts.find('T') {
            let after_t = &ts[t_pos + 1..];
            let end = after_t.len().min(12);
            &after_t[..end]
        } else {
            ts
        }
    }

    /// Handle scroll in logs view.
    pub fn scroll(state: &mut AppState, delta: i16) {
        if delta < 0 {
            state.log_scroll = state.log_scroll.saturating_add(delta.unsigned_abs());
        } else {
            state.log_scroll = state.log_scroll.saturating_sub(delta as u16);
        }
    }
}

impl TuiWidget for LogsWidget {
    fn id(&self) -> &str {
        "logs"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        // Title line with active filter
        let filter_label = if state.log_level_filter == crate::widgets::LogLevelFilter::All {
            String::new()
        } else {
            format!(" [{}]", state.log_level_filter)
        };
        let title = format!(
            " Logs ({} entries){filter_label} \u{2500} 1-5 filter \u{2500} Ctrl-L to return ",
            state.log_entries.len()
        );
        let title_line = Line::from(Span::styled(title, self.theme.accent_style()));

        // Build log lines
        let usable_width = area.width as usize;
        let mut all_lines: Vec<Line<'_>> = vec![title_line];

        for entry in state
            .log_entries
            .iter()
            .filter(|e| state.log_level_filter.accepts(&e.level))
        {
            let ts = Self::short_timestamp(&entry.timestamp);
            let level_style = self.level_style(&entry.level);

            // Truncate message to fit in available width
            let prefix_len = ts.len() + 1 + 5 + 1; // "HH:MM:SS.mmm LEVEL "
            let msg_width = usable_width.saturating_sub(prefix_len + 2);
            let message = if entry.message.len() > msg_width {
                format!("{}...", &entry.message[..msg_width.saturating_sub(3)])
            } else {
                entry.message.clone()
            };

            let line = Line::from(vec![
                Span::styled(format!(" {ts} "), self.theme.dim_style()),
                Span::styled(format!("{:<5} ", entry.level), level_style),
                Span::styled(message, Style::default().fg(self.theme.fg.to_color())),
            ]);
            all_lines.push(line);
        }

        // Compute visible window (scroll from bottom)
        let visible_height = area.height as usize;
        let total_lines = all_lines.len();
        let scroll = state.log_scroll as usize;
        let start = total_lines.saturating_sub(visible_height + scroll);
        let end = total_lines.saturating_sub(scroll).min(total_lines);

        let visible: Vec<Line<'_>> = all_lines
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();

        let paragraph = ratatui::widgets::Paragraph::new(visible);
        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_timestamp_extracts_time() {
        assert_eq!(
            LogsWidget::short_timestamp("2024-01-01T08:00:36.095Z"),
            "08:00:36.095"
        );
    }

    #[test]
    fn short_timestamp_no_t_returns_full() {
        assert_eq!(LogsWidget::short_timestamp("no-t-here"), "no-t-here");
    }

    #[test]
    fn short_timestamp_short_after_t() {
        assert_eq!(LogsWidget::short_timestamp("xT12:00"), "12:00");
    }
}
