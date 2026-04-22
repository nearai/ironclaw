//! Placeholder widget for top-level surfaces that have shell/navigation wiring
//! before their full panel implementations land.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::theme::Theme;

use super::ActiveTab;

pub struct SurfacePlaceholderWidget {
    theme: Theme,
}

impl SurfacePlaceholderWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub fn render_surface(&self, area: Rect, buf: &mut Buffer, active_tab: ActiveTab) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let lines = vec![
            Line::from(vec![
                Span::styled("◈ ", self.theme.accent_style()),
                Span::styled(
                    active_tab.title(),
                    self.theme.accent_style().add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                "WebGUI-parity shell scaffold is live for this surface.",
                Style::default().fg(self.theme.fg.to_color()),
            )),
            Line::from(Span::styled(
                "Detailed panels, data contracts, and drill-down flows land next.",
                self.theme.dim_style(),
            )),
        ];

        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style())
                    .title(format!(" {} ", active_tab.title())),
            )
            .style(Style::default().fg(self.theme.fg.to_color()))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_text(buf: &Buffer, area: Rect) -> String {
        let mut lines = Vec::new();
        for y in area.top()..area.bottom() {
            let mut line = String::new();
            for x in area.left()..area.right() {
                line.push_str(buf[(x, y)].symbol());
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    #[test]
    fn placeholder_renders_surface_title() {
        let widget = SurfacePlaceholderWidget::new(Theme::dark());
        let area = Rect::new(0, 0, 80, 8);
        let mut buf = Buffer::empty(area);

        widget.render_surface(area, &mut buf, ActiveTab::Workspace);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Workspace"));
        assert!(text.contains("shell scaffold"));
    }
}
