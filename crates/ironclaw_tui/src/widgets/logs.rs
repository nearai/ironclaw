//! Logs widget: scrollable log viewer with color-coded levels and filter controls.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, LogLevelFilter, TuiWidget};

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

    /// Compact level indicator (single char + color).
    fn level_indicator(level: &str) -> &'static str {
        match level {
            "ERROR" => "\u{25CF}", // ●
            "WARN" => "\u{25B2}",  // ▲
            "INFO" => "\u{25C6}",  // ◆
            "DEBUG" => "\u{25CB}", // ○
            "TRACE" => "\u{00B7}", // ·
            _ => " ",
        }
    }

    /// Shorten an ISO 8601 timestamp to HH:MM:SS.mmm for density.
    fn short_timestamp(ts: &str) -> &str {
        // Input: "2024-01-01T08:00:36.095Z" → want "08:00:36.095"
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

    /// Build the filter bar showing active filter with level selectors.
    fn render_filter_bar<'a>(&self, state: &AppState, width: usize) -> Line<'a> {
        let filters = [
            (LogLevelFilter::All, "All"),
            (LogLevelFilter::Error, "ERR"),
            (LogLevelFilter::Warn, "WRN"),
            (LogLevelFilter::Info, "INF"),
            (LogLevelFilter::Debug, "DBG"),
        ];

        let mut spans: Vec<Span<'a>> = Vec::with_capacity(filters.len() * 3 + 6);
        spans.push(Span::styled(
            " \u{25B8} Filter: ",
            self.theme.accent_style().add_modifier(Modifier::BOLD),
        ));

        for (i, (filter, label)) in filters.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", self.theme.dim_style()));
            }
            let key = format!("{}", i + 1);
            if state.log_level_filter == *filter {
                // Active filter: highlighted
                spans.push(Span::styled(
                    format!("[{key}:{label}]"),
                    self.theme
                        .accent_style()
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" {key}:{label} "),
                    self.theme.dim_style(),
                ));
            }
        }

        // Right side: entry count and scroll position
        let filtered_count = state
            .log_entries
            .iter()
            .filter(|e| state.log_level_filter.accepts(&e.level))
            .count();
        let count_text = format!("  {filtered_count} entries ");
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let remaining = width.saturating_sub(used + count_text.chars().count());
        if remaining > 0 {
            spans.push(Span::raw(" ".repeat(remaining)));
        }
        spans.push(Span::styled(count_text, self.theme.dim_style()));

        Line::from(spans)
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

        let usable_width = area.width as usize;

        // Filter bar at the top
        let filter_bar = self.render_filter_bar(state, usable_width);

        // Separator line
        let separator = Line::from(Span::styled(
            " ".to_string() + &"\u{2500}".repeat(usable_width.saturating_sub(2)) + " ",
            self.theme.dim_style(),
        ));

        let mut all_lines: Vec<Line<'_>> = vec![filter_bar, separator];

        // Build log lines with compact indicators
        for entry in state
            .log_entries
            .iter()
            .filter(|e| state.log_level_filter.accepts(&e.level))
        {
            let ts = Self::short_timestamp(&entry.timestamp);
            let level_style = self.level_style(&entry.level);
            let indicator = Self::level_indicator(&entry.level);

            // Truncate message to fit in available width
            let prefix_len = 1 + 2 + ts.len() + 1 + 1; // " ● HH:MM:SS.mmm "
            let msg_width = usable_width.saturating_sub(prefix_len + 1);
            let message = if entry.message.len() > msg_width {
                format!("{}...", &entry.message[..msg_width.saturating_sub(3)])
            } else {
                entry.message.clone()
            };

            let line = Line::from(vec![
                Span::styled(format!(" {indicator} "), level_style),
                Span::styled(format!("{ts} "), self.theme.dim_style()),
                Span::styled(message, Style::default().fg(self.theme.fg.to_color())),
            ]);
            all_lines.push(line);
        }

        // Empty state
        if all_lines.len() <= 2 {
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(Span::styled(
                "  No log entries match the current filter.",
                self.theme.dim_style(),
            )));
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

    #[test]
    fn level_indicator_maps() {
        assert_eq!(LogsWidget::level_indicator("ERROR"), "\u{25CF}");
        assert_eq!(LogsWidget::level_indicator("WARN"), "\u{25B2}");
        assert_eq!(LogsWidget::level_indicator("INFO"), "\u{25C6}");
        assert_eq!(LogsWidget::level_indicator("DEBUG"), "\u{25CB}");
    }

    #[test]
    fn logs_renders_without_panic() {
        let theme = Theme::dark();
        let widget = LogsWidget::new(theme);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
    }
}
