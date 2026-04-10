//! Tab bar widget: shows [Chat] [Logs] tabs with the active one highlighted.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

pub struct TabBarWidget {
    theme: Theme,
}

impl TabBarWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for TabBarWidget {
    fn id(&self) -> &str {
        "tab_bar"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 12 {
            return;
        }

        let (chat_style, logs_style) = match state.active_tab {
            ActiveTab::Conversation => (
                self.theme
                    .accent_style()
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                self.theme.dim_style(),
            ),
            ActiveTab::Logs => (
                self.theme.dim_style(),
                self.theme
                    .accent_style()
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ),
        };

        let line = Line::from(vec![
            Span::styled("  ", self.theme.dim_style()),
            Span::styled("Chat", chat_style),
            Span::styled("  ", self.theme.dim_style()),
            Span::styled("Logs", logs_style),
            Span::styled("  ", self.theme.dim_style()),
        ]);

        let paragraph = ratatui::widgets::Paragraph::new(line);
        paragraph.render(area, buf);
    }
}
