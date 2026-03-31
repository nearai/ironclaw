//! Slash command palette overlay widget.
//!
//! When the user types `/` in the input box, a filtered list of available
//! slash commands appears above the input area. Arrow keys navigate,
//! Enter/Tab selects, Esc dismisses.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::theme::Theme;

/// A slash command entry.
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

/// Static list of known slash commands.
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/help",
        description: "Show this help",
    },
    SlashCommand {
        name: "/model",
        description: "Show or switch the active model",
    },
    SlashCommand {
        name: "/version",
        description: "Show version info",
    },
    SlashCommand {
        name: "/tools",
        description: "List available tools",
    },
    SlashCommand {
        name: "/debug",
        description: "Toggle debug mode",
    },
    SlashCommand {
        name: "/ping",
        description: "Connectivity check",
    },
    SlashCommand {
        name: "/job",
        description: "Create a new job",
    },
    SlashCommand {
        name: "/status",
        description: "Check job status",
    },
    SlashCommand {
        name: "/cancel",
        description: "Cancel a job",
    },
    SlashCommand {
        name: "/list",
        description: "List all jobs",
    },
    SlashCommand {
        name: "/undo",
        description: "Undo last turn",
    },
    SlashCommand {
        name: "/redo",
        description: "Redo undone turn",
    },
    SlashCommand {
        name: "/compact",
        description: "Compress context window",
    },
    SlashCommand {
        name: "/clear",
        description: "Clear current thread",
    },
    SlashCommand {
        name: "/interrupt",
        description: "Stop current operation",
    },
    SlashCommand {
        name: "/new",
        description: "New conversation thread",
    },
    SlashCommand {
        name: "/skills",
        description: "List installed skills",
    },
    SlashCommand {
        name: "/heartbeat",
        description: "Run heartbeat check",
    },
    SlashCommand {
        name: "/summarize",
        description: "Summarize current thread",
    },
    SlashCommand {
        name: "/suggest",
        description: "Suggest next steps",
    },
    SlashCommand {
        name: "/quit",
        description: "Exit",
    },
];

/// Mutable state for the command palette.
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    /// Whether the palette is visible.
    pub visible: bool,
    /// Text after the leading `/`, used for filtering.
    pub filter: String,
    /// Indices into [`COMMANDS`] that match the current filter.
    pub filtered: Vec<usize>,
    /// Index into `filtered` for the currently highlighted row.
    pub selected: usize,
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self {
            visible: false,
            filter: String::new(),
            filtered: (0..COMMANDS.len()).collect(),
            selected: 0,
        }
    }
}

impl CommandPaletteState {
    /// Recompute `filtered` from the current `filter` text.
    pub fn update_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        let lower = filter.to_lowercase();
        self.filtered = COMMANDS
            .iter()
            .enumerate()
            .filter(|(_, cmd)| cmd.name[1..].starts_with(&lower))
            .map(|(i, _)| i)
            .collect();
        // Clamp selected
        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    /// Move selection up.
    pub fn move_up(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Move selection down.
    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    /// Get the name of the currently selected command, if any.
    pub fn selected_command(&self) -> Option<&'static str> {
        self.filtered
            .get(self.selected)
            .map(|&idx| COMMANDS[idx].name)
    }

    /// Open the palette and reset state.
    pub fn open(&mut self, filter: &str) {
        self.visible = true;
        self.update_filter(filter);
    }

    /// Close the palette.
    pub fn close(&mut self) {
        self.visible = false;
        self.filter.clear();
        self.filtered = (0..COMMANDS.len()).collect();
        self.selected = 0;
    }
}

/// Renders the command palette overlay.
pub struct CommandPaletteWidget {
    theme: Theme,
}

impl CommandPaletteWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Maximum number of visible rows.
    const MAX_VISIBLE: usize = 12;

    /// Compute the overlay area positioned directly above the input area.
    ///
    /// `input_area` is the full input row (including border). The palette
    /// floats above it within `terminal`.
    pub fn palette_area(terminal: Rect, input_area: Rect, item_count: usize) -> Rect {
        let rows = (item_count).min(Self::MAX_VISIBLE) as u16;
        if rows == 0 {
            return Rect::default();
        }
        // 1 row top border + rows + 1 row bottom border
        let height = rows + 2;
        let y = input_area.y.saturating_sub(height);
        Rect::new(terminal.x, y, terminal.width, height)
    }

    /// Render the palette into the given area using the provided state.
    pub fn render_palette(
        &self,
        area: Rect,
        buf: &mut Buffer,
        palette_state: &CommandPaletteState,
    ) {
        if area.height < 3 || area.width < 10 || palette_state.filtered.is_empty() {
            return;
        }

        // Clear background
        Clear.render(area, buf);

        // Draw top border
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_symbol("\u{2500}");
                cell.set_style(self.theme.border_style());
            }
        }

        // Draw bottom border
        let bottom = area.y + area.height - 1;
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, bottom)) {
                cell.set_symbol("\u{2500}");
                cell.set_style(self.theme.border_style());
            }
        }

        // Content area (between borders)
        let content_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };

        if content_area.width < 8 {
            return;
        }

        // Figure out the visible window of items
        let total = palette_state.filtered.len();
        let max_visible = content_area.height as usize;

        // Scroll so selected item is always visible
        let scroll_offset = if palette_state.selected >= max_visible {
            palette_state.selected - max_visible + 1
        } else {
            0
        };

        let visible_range = scroll_offset..total.min(scroll_offset + max_visible);

        for (row_idx, filtered_idx) in visible_range.enumerate() {
            let cmd_idx = palette_state.filtered[filtered_idx];
            let cmd = &COMMANDS[cmd_idx];
            let is_selected = filtered_idx == palette_state.selected;

            let y = content_area.y + row_idx as u16;
            if y >= content_area.y + content_area.height {
                break;
            }

            // Build the line
            let name_style = if is_selected {
                self.theme.bold_accent_style()
            } else {
                self.theme.accent_style()
            };
            let desc_style = if is_selected {
                self.theme.dim_style().add_modifier(Modifier::BOLD)
            } else {
                self.theme.dim_style()
            };

            // Pad name to fixed width for alignment
            let name_width = 16usize.min(content_area.width as usize / 2);
            let padded_name = format!("  {:<width$}", cmd.name, width = name_width);
            let desc = cmd.description;

            let line = Line::from(vec![
                Span::styled(padded_name, name_style),
                Span::styled(desc, desc_style),
            ]);

            let line_area = Rect {
                x: content_area.x,
                y,
                width: content_area.width,
                height: 1,
            };

            // If selected, fill background
            if is_selected {
                for bx in line_area.x..line_area.x + line_area.width {
                    if let Some(cell) = buf.cell_mut((bx, y)) {
                        cell.set_style(
                            ratatui::style::Style::default().bg(self.theme.border.to_color()),
                        );
                    }
                }
            }

            ratatui::widgets::Paragraph::new(line).render(line_area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_prefix() {
        let mut state = CommandPaletteState::default();
        state.update_filter("he");
        // Should match /help and /heartbeat
        assert_eq!(state.filtered.len(), 2);
        let names: Vec<&str> = state.filtered.iter().map(|&i| COMMANDS[i].name).collect();
        assert!(names.contains(&"/help"));
        assert!(names.contains(&"/heartbeat"));
    }

    #[test]
    fn filter_empty_matches_all() {
        let mut state = CommandPaletteState::default();
        state.update_filter("");
        assert_eq!(state.filtered.len(), COMMANDS.len());
    }

    #[test]
    fn filter_no_match() {
        let mut state = CommandPaletteState::default();
        state.update_filter("zzzzz");
        assert!(state.filtered.is_empty());
    }

    #[test]
    fn move_up_wraps() {
        let mut state = CommandPaletteState::default();
        state.update_filter("");
        state.selected = 0;
        state.move_up();
        assert_eq!(state.selected, state.filtered.len() - 1);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = CommandPaletteState::default();
        state.update_filter("");
        state.selected = state.filtered.len() - 1;
        state.move_down();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn selected_command_returns_name() {
        let mut state = CommandPaletteState::default();
        state.update_filter("");
        state.selected = 0;
        assert_eq!(state.selected_command(), Some("/help"));
    }

    #[test]
    fn open_and_close() {
        let mut state = CommandPaletteState::default();
        state.open("he");
        assert!(state.visible);
        assert_eq!(state.filter, "he");
        assert_eq!(state.filtered.len(), 2);

        state.close();
        assert!(!state.visible);
        assert_eq!(state.filtered.len(), COMMANDS.len());
    }
}
