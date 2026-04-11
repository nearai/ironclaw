//! Header bar widget: branding, model info, session duration, live activity indicators.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::{format_duration, format_tokens};
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub struct HeaderWidget {
    theme: Theme,
}

impl HeaderWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for HeaderWidget {
    fn id(&self) -> &str {
        "header"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Header
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let elapsed = chrono::Utc::now()
            .signed_duration_since(state.session_start)
            .num_seconds()
            .unsigned_abs();
        let duration = format_duration(elapsed);
        let tokens = format_tokens(state.total_input_tokens + state.total_output_tokens);

        let sep = Span::styled("  \u{00B7}  ", self.theme.dim_style());

        let mut spans = vec![
            Span::styled(
                format!("  ironclaw v{}", state.version),
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            sep.clone(),
            Span::styled(state.model.clone(), self.theme.bold_style()),
            sep.clone(),
            Span::styled(duration, self.theme.dim_style()),
        ];

        let total = state.total_input_tokens + state.total_output_tokens;
        if total > 0 {
            spans.push(sep.clone());
            spans.push(Span::styled(
                format!("{tokens} tokens"),
                self.theme.dim_style(),
            ));
        }

        // Streaming indicator
        if state.is_streaming {
            spans.push(sep.clone());
            let frame = state.tick_count % 3;
            let dots = ".".repeat(frame + 1);
            spans.push(Span::styled(
                format!("streaming{dots}"),
                self.theme.accent_style(),
            ));
        }

        // Active tool count
        if !state.active_tools.is_empty() {
            spans.push(sep);
            spans.push(Span::styled(
                format!("\u{26A1}{} active", state.active_tools.len()),
                self.theme.warning_style(),
            ));
        }

        let line = Line::from(spans);
        let widget = ratatui::widgets::Paragraph::new(line).style(self.theme.header_style());
        widget.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_renders_without_panic() {
        let theme = Theme::dark();
        let widget = HeaderWidget::new(theme);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
    }

    #[test]
    fn header_zero_area_skips() {
        let theme = Theme::dark();
        let widget = HeaderWidget::new(theme);
        let state = AppState::default();
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
    }
}
