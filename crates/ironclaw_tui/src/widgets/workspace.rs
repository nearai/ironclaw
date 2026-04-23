//! Workspace surface: searchable-ish file list and preview pane backed by
//! memory entries already available to the TUI.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::{render_markdown, truncate};
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub enum WorkspaceMouseAction {
    ToggleDirectory(String),
    SelectFile(String),
}

pub struct WorkspaceWidget {
    theme: Theme,
}

impl WorkspaceWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub fn pane_areas(area: Rect) -> (Rect, Rect) {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
            .split(area);
        (split[0], split[1])
    }

    pub fn list_rows(area: Rect) -> usize {
        area.height.saturating_sub(2) as usize
    }

    pub fn action_at(
        &self,
        area: Rect,
        state: &AppState,
        column: u16,
        row: u16,
    ) -> Option<WorkspaceMouseAction> {
        let (list_area, _) = Self::pane_areas(area);
        if !rect_contains(list_area, column, row)
            || row <= list_area.y
            || row >= list_area.bottom().saturating_sub(1)
        {
            return None;
        }

        let inner_y = row.saturating_sub(list_area.y + 1) as usize;
        let index = state.workspace.scroll + inner_y;
        let item = state
            .workspace
            .visible_items(&state.memory_entries)
            .get(index)?
            .clone();
        if item.is_dir {
            Some(WorkspaceMouseAction::ToggleDirectory(item.path))
        } else {
            Some(WorkspaceMouseAction::SelectFile(item.path))
        }
    }

    fn preview_lines(&self, state: &AppState, width: u16, height: u16) -> Vec<Line<'static>> {
        let Some(entry) = state.workspace.selected_entry(&state.memory_entries) else {
            return vec![Line::from(Span::styled(
                if state.memory_entries.is_empty() {
                    "No workspace files available yet."
                } else {
                    "Select a file to preview."
                },
                self.theme.dim_style(),
            ))];
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Workspace", self.theme.dim_style()),
                Span::styled("  /  ", self.theme.dim_style()),
                Span::styled(entry.path.clone(), self.theme.bold_style()),
            ]),
            Line::from(vec![
                Span::styled("Mode ", self.theme.dim_style()),
                Span::styled(
                    match state.workspace.preview_mode {
                        super::WorkspacePreviewMode::Rendered => "Rendered preview",
                        super::WorkspacePreviewMode::Raw => "Raw preview",
                    },
                    self.theme.accent_style(),
                ),
                Span::styled("  ·  ", self.theme.dim_style()),
                Span::styled("Enter toggles view", self.theme.dim_style()),
            ]),
        ];

        if let Some(updated_at) = entry.updated_at {
            lines.push(Line::from(vec![
                Span::styled("Updated ", self.theme.dim_style()),
                Span::styled(
                    updated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                    self.theme.dim_style(),
                ),
            ]));
        }

        lines.push(Line::from(""));

        let content_width = width.saturating_sub(2) as usize;
        let preview_lines = match state.workspace.preview_mode {
            super::WorkspacePreviewMode::Rendered => {
                render_markdown(&entry.snippet, content_width.max(1), &self.theme)
            }
            super::WorkspacePreviewMode::Raw => entry
                .snippet
                .lines()
                .map(|line| {
                    Line::from(Span::styled(
                        truncate(line, content_width.max(1)),
                        Style::default().fg(self.theme.fg.to_color()),
                    ))
                })
                .collect(),
        };
        if preview_lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "(empty preview)",
                self.theme.dim_style(),
            )));
        } else {
            lines.extend(preview_lines);
        }

        let usable_height = height.saturating_sub(2) as usize;
        let scroll = state
            .workspace
            .preview_scroll
            .min(lines.len().saturating_sub(usable_height));
        lines
            .into_iter()
            .skip(scroll)
            .take(usable_height.max(1))
            .collect()
    }
}

impl TuiWidget for WorkspaceWidget {
    fn id(&self) -> &str {
        "workspace"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::ConversationBanner
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 20 {
            return;
        }

        let (list_area, preview_area) = Self::pane_areas(area);

        let visible_rows = Self::list_rows(list_area);
        let visible_items = state.workspace.visible_items(&state.memory_entries);
        let list_lines: Vec<Line<'static>> = if visible_items.is_empty() {
            vec![Line::from(Span::styled(
                "No files in workspace",
                self.theme.dim_style(),
            ))]
        } else {
            visible_items
                .iter()
                .enumerate()
                .skip(state.workspace.scroll)
                .take(visible_rows.max(1))
                .map(|(index, item)| {
                    let is_selected = index == state.workspace.selected;
                    let prefix = if is_selected { "▶ " } else { "  " };
                    let indent = "  ".repeat(item.depth);
                    let icon = if item.is_dir {
                        if state.workspace.collapsed_dirs.contains(&item.path) {
                            "▸ "
                        } else {
                            "▾ "
                        }
                    } else {
                        "• "
                    };
                    let path_width =
                        list_area.width.saturating_sub(5 + (item.depth as u16 * 2)) as usize;
                    let style = if is_selected {
                        self.theme.accent_style().add_modifier(Modifier::BOLD)
                    } else if item.is_dir {
                        self.theme.dim_style().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.theme.fg.to_color())
                    };
                    Line::from(Span::styled(
                        format!(
                            "{prefix}{indent}{icon}{}",
                            truncate(&item.label, path_width.max(1))
                        ),
                        style,
                    ))
                })
                .collect()
        };

        Paragraph::new(list_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style())
                    .title(" Workspace browser "),
            )
            .render(list_area, buf);

        Paragraph::new(self.preview_lines(state, preview_area.width, preview_area.height))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style())
                    .title(" Preview "),
            )
            .render(preview_area, buf);
    }
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{MemoryEntry, WorkspacePreviewMode, WorkspaceState};

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
    fn workspace_renders_selected_entry_preview() {
        let widget = WorkspaceWidget::new(Theme::dark());
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let state = AppState {
            memory_entries: vec![
                MemoryEntry {
                    path: "docs/spec.md".to_string(),
                    snippet: "hello from preview pane".to_string(),
                    updated_at: None,
                },
                MemoryEntry {
                    path: "notes/todo.md".to_string(),
                    snippet: "other file".to_string(),
                    updated_at: None,
                },
            ],
            workspace: WorkspaceState {
                selected: 0,
                selected_path: Some("docs/spec.md".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Workspace browser"));
        assert!(text.contains("docs"));
        assert!(text.contains("spec.md"));
        assert!(text.contains("hello from preview pane"));
    }

    #[test]
    fn workspace_rendered_preview_uses_markdown_renderer() {
        let widget = WorkspaceWidget::new(Theme::dark());
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let state = AppState {
            memory_entries: vec![MemoryEntry {
                path: "docs/spec.md".to_string(),
                snippet: "# Title\n\n- alpha\n- beta".to_string(),
                updated_at: None,
            }],
            workspace: WorkspaceState {
                selected_path: Some("docs/spec.md".to_string()),
                preview_mode: WorkspacePreviewMode::Rendered,
                ..Default::default()
            },
            ..Default::default()
        };

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Title"));
        assert!(
            text.contains("• alpha"),
            "rendered mode should use markdown rendering for list items: {text}"
        );
        assert!(text.contains("• beta"));
    }

    #[test]
    fn workspace_raw_preview_preserves_line_breaks() {
        let widget = WorkspaceWidget::new(Theme::dark());
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let state = AppState {
            memory_entries: vec![MemoryEntry {
                path: "docs/spec.md".to_string(),
                snippet: "line one\nline two".to_string(),
                updated_at: None,
            }],
            workspace: WorkspaceState {
                selected_path: Some("docs/spec.md".to_string()),
                preview_mode: WorkspacePreviewMode::Raw,
                ..Default::default()
            },
            ..Default::default()
        };

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("line one"));
        assert!(text.contains("line two"));
    }

    #[test]
    fn workspace_state_sync_preserves_selected_path() {
        let mut workspace = WorkspaceState {
            selected: 1,
            selected_path: Some("b.md".to_string()),
            ..Default::default()
        };
        let entries = vec![
            MemoryEntry {
                path: "z.md".to_string(),
                snippet: String::new(),
                updated_at: None,
            },
            MemoryEntry {
                path: "b.md".to_string(),
                snippet: String::new(),
                updated_at: None,
            },
            MemoryEntry {
                path: "a.md".to_string(),
                snippet: String::new(),
                updated_at: None,
            },
        ];

        workspace.sync_entries(&entries);

        assert_eq!(workspace.selected, 1);
        assert_eq!(workspace.selected_path.as_deref(), Some("b.md"));
    }

    #[test]
    fn action_at_toggles_directory_rows() {
        let widget = WorkspaceWidget::new(Theme::dark());
        let area = Rect::new(0, 0, 100, 20);
        let state = AppState {
            memory_entries: vec![MemoryEntry {
                path: "docs/a.md".to_string(),
                snippet: String::new(),
                updated_at: None,
            }],
            workspace: WorkspaceState {
                selected_path: Some("docs".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let (list_area, _) = WorkspaceWidget::pane_areas(area);

        let action = widget.action_at(area, &state, list_area.x + 2, list_area.y + 1);

        assert!(matches!(
            action,
            Some(WorkspaceMouseAction::ToggleDirectory(path)) if path == "docs"
        ));
    }

    #[test]
    fn workspace_state_toggle_directory_hides_children() {
        let entries = vec![MemoryEntry {
            path: "docs/a.md".to_string(),
            snippet: String::new(),
            updated_at: None,
        }];
        let mut workspace = WorkspaceState {
            selected_path: Some("docs".to_string()),
            ..Default::default()
        };
        workspace.sync_entries(&entries);
        assert_eq!(workspace.visible_items(&entries).len(), 2);

        assert!(workspace.toggle_selected_directory(&entries));
        assert_eq!(workspace.visible_items(&entries).len(), 1);
    }
}
