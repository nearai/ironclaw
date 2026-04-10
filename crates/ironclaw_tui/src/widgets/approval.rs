//! Modal overlay for tool approval dialog.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

/// Approval action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAction {
    Approve,
    Always,
    Deny,
}

impl ApprovalAction {
    pub fn as_response(&self) -> &'static str {
        match self {
            Self::Approve => "y",
            Self::Always => "a",
            Self::Deny => "n",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Approve => "Approve (y)",
            Self::Always => "Always approve (a)",
            Self::Deny => "Deny (n)",
        }
    }
}

pub struct ApprovalWidget {
    theme: Theme,
}

impl ApprovalWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Get the list of options for the current approval request.
    pub fn options(allow_always: bool) -> Vec<ApprovalAction> {
        if allow_always {
            vec![
                ApprovalAction::Approve,
                ApprovalAction::Always,
                ApprovalAction::Deny,
            ]
        } else {
            vec![ApprovalAction::Approve, ApprovalAction::Deny]
        }
    }

    /// Compute the centered modal area within the terminal.
    pub fn modal_area(terminal: Rect) -> Rect {
        let width = terminal.width.clamp(30, 60);
        let height = terminal.height.clamp(8, 16);
        let x = terminal.x + (terminal.width.saturating_sub(width)) / 2;
        let y = terminal.y + (terminal.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl TuiWidget for ApprovalWidget {
    fn id(&self) -> &str {
        "approval"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let approval = match &state.pending_approval {
            Some(a) => a,
            None => return,
        };

        // Clear the area behind the modal
        Clear.render(area, buf);

        let block = Block::default()
            .title(" Tool Approval ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(self.theme.accent_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width < 4 {
            return;
        }

        let mut lines: Vec<Line<'_>> = Vec::new();

        // Tool name
        lines.push(Line::from(vec![
            Span::styled(
                format!(" \u{25C6} {} ", approval.tool_name),
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled("requires approval", self.theme.dim_style()),
        ]));
        lines.push(Line::from(""));

        // Parameters (simple key: value display)
        if let Some(obj) = approval.parameters.as_object() {
            for (key, value) in obj.iter().take(4) {
                let val_str = match value {
                    serde_json::Value::String(s) => {
                        if s.len() > 40 {
                            format!("\"{}...\"", &s[..37])
                        } else {
                            format!("\"{s}\"")
                        }
                    }
                    other => {
                        let rendered = other.to_string();
                        if rendered.len() > 40 {
                            format!("{}...", &rendered[..37])
                        } else {
                            rendered
                        }
                    }
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {key}: "), self.theme.accent_style()),
                    Span::styled(val_str, self.theme.dim_style()),
                ]));
            }
        }
        lines.push(Line::from(""));

        // Options
        let options = Self::options(approval.allow_always);
        for (i, opt) in options.iter().enumerate() {
            let (icon, style) = if i == approval.selected {
                ("\u{25CF}", self.theme.bold_accent_style())
            } else {
                ("\u{25CB}", self.theme.dim_style())
            };
            lines.push(Line::from(Span::styled(
                format!("  {icon} {}", opt.label()),
                style,
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  \u{2191}\u{2193} select  Enter confirm  Esc cancel",
            self.theme.dim_style(),
        )));

        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}
