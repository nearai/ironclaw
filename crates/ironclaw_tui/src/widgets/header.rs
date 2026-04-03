//! Header bar widget: version, model, session duration, token count.

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

        let mut spans = vec![
            Span::styled(
                format!("  ironclaw v{}", state.version),
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(state.model.clone(), self.theme.bold_style()),
            Span::styled("  ·  ", self.theme.dim_style()),
            Span::styled(duration, self.theme.dim_style()),
        ];

        let total = state.total_input_tokens + state.total_output_tokens;
        if total > 0 {
            spans.push(Span::styled("  ·  ", self.theme.dim_style()));
            spans.push(Span::styled(
                format!("{tokens} tokens"),
                self.theme.dim_style(),
            ));
        }

        let line = Line::from(spans);
        let widget = ratatui::widgets::Paragraph::new(line).style(self.theme.header_style());
        widget.render(area, buf);
    }
}
