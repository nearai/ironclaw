//! Tool execution sidebar panel.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::truncate;
use crate::theme::Theme;

use super::{AppState, ToolStatus, TuiWidget};

pub struct ToolPanelWidget {
    theme: Theme,
}

impl ToolPanelWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for ToolPanelWidget {
    fn id(&self) -> &str {
        "tool_panel"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Sidebar
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        let max_name_len = (area.width as usize).saturating_sub(12);
        let mut lines: Vec<Line<'_>> = Vec::new();

        // Section header
        let active_count = state.active_tools.len();
        let total_count = active_count + state.recent_tools.len();
        lines.push(Line::from(vec![
            Span::styled(" Tools", self.theme.bold_style()),
            Span::styled(
                format!("  {active_count}/{total_count}"),
                self.theme.dim_style(),
            ),
        ]));

        let show_detail = area.width >= 12;

        // Active tools
        for tool in &state.active_tools {
            let elapsed = chrono::Utc::now()
                .signed_duration_since(tool.started_at)
                .num_milliseconds()
                .unsigned_abs();
            let name = truncate(&tool.name, max_name_len);
            lines.push(Line::from(vec![
                Span::styled(" \u{25CF} ", self.theme.accent_style()),
                Span::styled(name, self.theme.accent_style()),
                Span::styled(format!("  {elapsed}ms"), self.theme.dim_style()),
            ]));
            if show_detail && let Some(ref d) = tool.detail {
                let detail_max = (area.width as usize).saturating_sub(4);
                lines.push(Line::from(Span::styled(
                    format!("   {}", truncate(d, detail_max)),
                    self.theme.dim_style(),
                )));
            }
        }

        // Recent tools
        for (recent_shown, tool) in state
            .recent_tools
            .iter()
            .rev()
            .take(
                (area.height as usize)
                    .saturating_sub(lines.len())
                    .saturating_sub(1),
            )
            .enumerate()
        {
            let name = truncate(&tool.name, max_name_len);
            let (icon, style) = match tool.status {
                ToolStatus::Success => ("\u{25CF}", self.theme.success_style()),
                ToolStatus::Failed => ("\u{2717}", self.theme.error_style()),
                ToolStatus::Running => ("\u{25CB}", self.theme.accent_style()),
            };
            let duration_text = tool
                .duration_ms
                .map(|d| format!("  {d}ms"))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(name, self.theme.dim_style()),
                Span::styled(duration_text, self.theme.dim_style()),
            ]));
            // Show detail for the 3 most recent completed tools
            if show_detail
                && recent_shown < 3
                && let Some(d) = tool.result_preview.as_deref().or(tool.detail.as_deref())
            {
                let detail_max = (area.width as usize).saturating_sub(4);
                lines.push(Line::from(Span::styled(
                    format!("   {}", truncate(d, detail_max)),
                    self.theme.dim_style(),
                )));
            }
        }

        let paragraph = ratatui::widgets::Paragraph::new(lines);
        paragraph.render(area, buf);
    }
}
