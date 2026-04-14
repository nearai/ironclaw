//! Help overlay: keybinding reference modal with categorized sections.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

/// Keybinding section with a category header and entries.
struct KeySection {
    header: &'static str,
    bindings: &'static [(&'static str, &'static str)],
}

/// All keybindings grouped by category.
const KEY_SECTIONS: &[KeySection] = &[
    KeySection {
        header: "Navigation",
        bindings: &[
            ("Ctrl-L", "Cycle tabs"),
            ("Ctrl-B", "Toggle dashboard"),
            ("Ctrl-F", "Search conversation"),
            ("PgUp / PgDn", "Scroll"),
        ],
    },
    KeySection {
        header: "Input",
        bindings: &[
            ("Enter", "Submit message"),
            ("Up / Down", "Input history"),
            ("Ctrl-P / Ctrl-N", "Input history (alt)"),
            ("Ctrl-V", "Paste image from clipboard"),
        ],
    },
    KeySection {
        header: "Tools & Actions",
        bindings: &[
            ("Ctrl-E", "Expand tool output"),
            ("y / n / a", "Approval shortcuts"),
            ("Mouse drag", "Select text and copy"),
        ],
    },
    KeySection {
        header: "System",
        bindings: &[
            ("Ctrl-/", "Toggle this help"),
            ("Ctrl-O", "Toggle work sidebar"),
            ("1-5", "Log level filter (Logs tab)"),
            ("Esc", "Interrupt / cancel"),
            ("Ctrl-C", "Quit"),
        ],
    },
];

/// Total number of lines the help content will occupy (headers + bindings + spacing).
fn total_content_lines() -> usize {
    let mut lines = 0;
    for (i, section) in KEY_SECTIONS.iter().enumerate() {
        if i > 0 {
            lines += 1; // blank line between sections
        }
        lines += 1; // section header
        lines += section.bindings.len();
    }
    lines += 2; // footer spacing + hint
    lines
}

pub struct HelpOverlayWidget {
    theme: Theme,
}

impl HelpOverlayWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Compute the modal area centered in the terminal.
    pub fn modal_area(size: Rect) -> Rect {
        let width = 56u16.min(size.width.saturating_sub(4));
        let height = (total_content_lines() as u16 + 4).min(size.height.saturating_sub(4));
        let x = (size.width.saturating_sub(width)) / 2;
        let y = (size.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl TuiWidget for HelpOverlayWidget {
    fn id(&self) -> &str {
        "help_overlay"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _state: &AppState) {
        if area.height < 4 || area.width < 20 {
            return;
        }

        // Clear the area behind the modal
        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.accent_style())
            .title(Span::styled(
                " Keyboard Shortcuts ",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width < 10 {
            return;
        }

        let key_width = 18usize;
        let mut lines: Vec<Line<'_>> = Vec::with_capacity(total_content_lines() + 2);

        let section_header_style = self
            .theme
            .accent_style()
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        let key_style = Style::default()
            .fg(self.theme.accent.to_color())
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(self.theme.fg.to_color());

        for (i, section) in KEY_SECTIONS.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(""));
            }

            // Section header
            lines.push(Line::from(Span::styled(
                format!("  \u{25B8} {}", section.header),
                section_header_style,
            )));

            for (key, desc) in section.bindings {
                let padded_key = format!("    {key:<width$}", width = key_width);
                lines.push(Line::from(vec![
                    Span::styled(padded_key, key_style),
                    Span::styled(*desc, desc_style),
                ]));
            }
        }

        // Footer hint
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press Ctrl-/ or Esc to close",
            self.theme.dim_style(),
        )));

        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_overlay_renders_without_panic() {
        let theme = Theme::dark();
        let widget = HelpOverlayWidget::new(theme);
        let state = AppState::default();
        let area = Rect::new(5, 5, 56, 30);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
        widget.render(area, &mut buf, &state);
    }

    #[test]
    fn modal_area_centered() {
        let size = Rect::new(0, 0, 120, 40);
        let modal = HelpOverlayWidget::modal_area(size);
        assert!(modal.x > 0);
        assert!(modal.y > 0);
        assert!(modal.width <= 56);
    }

    #[test]
    fn total_content_lines_nonzero() {
        assert!(total_content_lines() > 10);
    }
}
