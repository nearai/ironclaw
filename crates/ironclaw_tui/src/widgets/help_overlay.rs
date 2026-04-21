//! Help overlay: F1 keybinding reference modal.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

/// Keybinding entries shown in the help overlay.
const KEYBINDINGS: &[(&str, &str)] = &[
    ("F1", "Toggle this help"),
    ("Enter", "Submit message"),
    ("Shift-Enter", "Insert newline"),
    ("Ctrl-C", "Quit"),
    ("Ctrl-B", "Toggle sidebar"),
    ("Ctrl-L", "Toggle logs"),
    ("Ctrl-F", "Search conversation"),
    ("Up / Down", "Input history at input edges"),
    ("Ctrl-P / Ctrl-N", "Input history"),
    ("Ctrl-E", "Expand tool output"),
    ("Ctrl-V", "Paste image from clipboard"),
    ("Mouse drag", "Select visible text and copy"),
    ("PgUp / PgDn", "Scroll"),
    ("Esc", "Interrupt / cancel"),
    ("y / n / a", "Approval shortcuts"),
    ("1-5", "Log level filter (Logs tab)"),
];

pub struct HelpOverlayWidget {
    theme: Theme,
}

impl HelpOverlayWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Compute the modal area centered in the terminal.
    pub fn modal_area(size: Rect) -> Rect {
        let width = 52u16.min(size.width.saturating_sub(4));
        let height = (KEYBINDINGS.len() as u16 + 4).min(size.height.saturating_sub(4));
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
        let mut lines: Vec<Line<'_>> = Vec::with_capacity(KEYBINDINGS.len() + 2);

        // Blank line for spacing
        lines.push(Line::from(""));

        for (key, desc) in KEYBINDINGS {
            let padded_key = format!("  {key:<width$}", width = key_width);
            lines.push(Line::from(vec![
                Span::styled(
                    padded_key,
                    self.theme.accent_style().add_modifier(Modifier::BOLD),
                ),
                Span::styled(*desc, Style::default().fg(self.theme.fg.to_color())),
            ]));
        }

        // Footer hint
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press F1 or Esc to close",
            self.theme.dim_style(),
        )));

        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}
