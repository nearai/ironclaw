//! Thread list sidebar panel.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::{format_duration, truncate};
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub struct ThreadListWidget {
    theme: Theme,
}

impl ThreadListWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for ThreadListWidget {
    fn id(&self) -> &str {
        "thread_list"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::SidebarSection
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        let max_label_len = (area.width as usize).saturating_sub(16);
        let mut lines: Vec<Line<'_>> = Vec::new();

        // Section header
        lines.push(Line::from(Span::styled(
            " Threads",
            self.theme.bold_style(),
        )));

        for thread in &state.threads {
            let short_id = if thread.id.len() > 3 {
                &thread.id[..3]
            } else {
                &thread.id
            };
            let kind = if thread.is_foreground { "fg" } else { "bg" };
            let dur = format_duration(thread.duration_secs);
            let label = truncate(&thread.label, max_label_len);

            let (icon, style) = if thread.is_running {
                ("\u{25CF}", self.theme.accent_style())
            } else {
                ("\u{2713}", self.theme.success_style())
            };

            let status = if thread.is_running { "running" } else { "done" };

            lines.push(Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(format!("#{short_id}"), self.theme.dim_style()),
                Span::styled(format!(" ({kind}) "), self.theme.dim_style()),
                Span::styled(format!("{status} {dur}"), self.theme.dim_style()),
            ]));

            if !label.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("   {label}"),
                    self.theme.dim_style(),
                )));
            }
        }

        if state.threads.is_empty() {
            lines.push(Line::from(Span::styled(
                " (no threads)",
                self.theme.dim_style(),
            )));
        }

        let visible: Vec<Line<'_>> = lines.into_iter().take(area.height as usize).collect();
        let paragraph = ratatui::widgets::Paragraph::new(visible);
        paragraph.render(area, buf);
    }
}
