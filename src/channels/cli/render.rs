//! TUI rendering with Ratatui.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::channels::cli::app::{AppState, InputMode, MessageRole, MessageStatus};
use crate::channels::cli::model_selector::ModelSelectorOverlay;
use crate::channels::cli::overlay::ApprovalSelection;

/// Render the entire UI.
pub fn render(frame: &mut Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // Messages
            Constraint::Length(3), // Input
            Constraint::Length(1), // Status
        ])
        .split(frame.area());

    render_messages(frame, app, chunks[0]);
    render_input(frame, app, chunks[1]);
    render_status(frame, app, chunks[2]);

    // Render approval overlay if active
    if app.mode == InputMode::Approval {
        render_approval_overlay(frame, app);
    }
}

/// Render the message history.
fn render_messages(frame: &mut Frame, app: &AppState, area: Rect) {
    // Build all lines from all messages
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        let (prefix, style) = match msg.role {
            MessageRole::User => ("You: ", Style::default().fg(Color::Cyan)),
            MessageRole::Agent => ("Agent: ", Style::default().fg(Color::Green)),
            MessageRole::System => (
                "",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        };

        let status_indicator = match msg.status {
            Some(MessageStatus::Pending) => " ⏳",
            Some(MessageStatus::InProgress) => " ⚙️",
            Some(MessageStatus::Complete) => " ✓",
            Some(MessageStatus::Error) => " ✗",
            None => "",
        };

        // Split content by newlines and create a line for each
        let content_lines: Vec<&str> = msg.content.lines().collect();
        for (i, line_text) in content_lines.iter().enumerate() {
            if i == 0 {
                // First line gets the prefix
                let line_content = if status_indicator.is_empty() {
                    format!("{}{}", prefix, line_text)
                } else if content_lines.len() == 1 {
                    format!("{}{}{}", prefix, line_text, status_indicator)
                } else {
                    format!("{}{}", prefix, line_text)
                };
                lines.push(Line::styled(line_content, style));
            } else if i == content_lines.len() - 1 && !status_indicator.is_empty() {
                // Last line gets status indicator
                lines.push(Line::styled(
                    format!("{}{}", line_text, status_indicator),
                    style,
                ));
            } else {
                // Middle lines just get the content
                lines.push(Line::styled(line_text.to_string(), style));
            }
        }

        // Add empty line between messages for readability
        lines.push(Line::from(""));
    }

    // Calculate scroll - show most recent messages
    let visible_height = area.height.saturating_sub(2) as usize; // Account for borders
    let total_lines = lines.len();
    let scroll_offset = total_lines.saturating_sub(visible_height);

    let text = Text::from(lines);
    let messages = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Chat"))
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset as u16, 0));

    frame.render_widget(messages, area);
}

/// Render the input area (or model selector when in ModelSelector mode).
fn render_input(frame: &mut Frame, app: &AppState, area: Rect) {
    // In ModelSelector mode, render inline selector instead of input
    if app.mode == InputMode::ModelSelector {
        render_model_selector_inline(frame, app, area);
        return;
    }

    let input_style = match app.mode {
        InputMode::Editing => Style::default().fg(Color::Yellow),
        InputMode::Normal => Style::default(),
        InputMode::Approval | InputMode::ModelSelector => Style::default().fg(Color::DarkGray),
    };

    let buffer = app.composer.buffer();
    let cursor = app.composer.cursor();

    // Build the input text with cursor
    let (before, after) = buffer.split_at(cursor.min(buffer.len()));
    let cursor_char = after.chars().next().unwrap_or(' ');
    let after_cursor = if after.is_empty() {
        ""
    } else {
        &after[cursor_char.len_utf8()..]
    };

    let input = Paragraph::new(Line::from(vec![
        Span::raw(before),
        Span::styled(
            cursor_char.to_string(),
            Style::default().bg(Color::White).fg(Color::Black),
        ),
        Span::raw(after_cursor),
    ]))
    .style(input_style)
    .block(Block::default().borders(Borders::ALL).title("Input"));

    frame.render_widget(input, area);

    // Show cursor in editing mode
    if app.mode == InputMode::Editing {
        // Calculate cursor position accounting for the block border
        let cursor_x = area.x + 1 + cursor as u16;
        let cursor_y = area.y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Render inline model selector in the input area.
fn render_model_selector_inline(frame: &mut Frame, app: &AppState, area: Rect) {
    let Some(ref overlay) = app.model_selector else {
        return;
    };

    let models = overlay.models();

    // Build horizontal list of models
    let mut spans: Vec<Span> = Vec::new();

    if models.is_empty() {
        spans.push(Span::styled(
            "Loading models...",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ));
    } else {
        for (i, model) in models.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }

            let display_name = ModelSelectorOverlay::format_model_name(model);
            let is_selected = i == overlay.selection_index;
            let is_current = model == &overlay.request.current_model;

            let style = if is_selected {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else if is_current {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_current { "●" } else { " " };
            spans.push(Span::styled(format!("{}{}", prefix, display_name), style));
        }
    }

    let content = Paragraph::new(Line::from(spans))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled("Select Model", Style::default().fg(Color::Cyan))),
        )
        .scroll((0, calculate_model_scroll(overlay, area.width.saturating_sub(2))));

    frame.render_widget(content, area);
}

/// Calculate horizontal scroll offset to keep selected model visible.
fn calculate_model_scroll(overlay: &ModelSelectorOverlay, visible_width: u16) -> u16 {
    let models = overlay.models();
    if models.is_empty() {
        return 0;
    }

    // Estimate position of selected model (rough calculation)
    let mut pos: u16 = 0;
    for (i, model) in models.iter().enumerate() {
        let name_len = ModelSelectorOverlay::format_model_name(model).len() as u16 + 3; // +3 for prefix and spacing
        if i == overlay.selection_index {
            // Check if selection is beyond visible area
            if pos > visible_width {
                return pos.saturating_sub(visible_width / 2);
            }
            return 0;
        }
        pos += name_len;
    }
    0
}

/// Render the status line.
fn render_status(frame: &mut Frame, app: &AppState, area: Rect) {
    let status_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        match app.mode {
            InputMode::Normal | InputMode::Editing => {
                let model = ModelSelectorOverlay::format_model_name(&app.current_model);
                format!("{} | /model to switch", model)
            }
            InputMode::Approval => "y=Yes, n=No, a=Always, Ctrl+C=Cancel".to_string(),
            InputMode::ModelSelector => "←/→ navigate, Enter=select, Esc=cancel".to_string(),
        }
    };

    let status = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));

    frame.render_widget(status, area);
}

/// Render the approval overlay.
fn render_approval_overlay(frame: &mut Frame, app: &AppState) {
    let Some(ref overlay) = app.approval else {
        return;
    };

    let area = frame.area();

    // Calculate overlay size and position
    let overlay_width = (area.width * 60 / 100).min(60);
    let overlay_height = 12;
    let overlay_x = (area.width - overlay_width) / 2;
    let overlay_y = (area.height - overlay_height) / 2;

    let overlay_area = Rect::new(overlay_x, overlay_y, overlay_width, overlay_height);

    // Clear the area behind the overlay
    frame.render_widget(Clear, overlay_area);

    // Build overlay content
    let title = if overlay.request.destructive {
        "⚠️  Approval Required (Destructive)"
    } else {
        "Approval Required"
    };

    let title_style = if overlay.request.destructive {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };

    // Build the text content
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Tool: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&overlay.request.tool_name),
        ]),
        Line::from(""),
        Line::from(overlay.request.description.as_str()),
        Line::from(""),
    ];

    // Add parameters preview (truncated)
    let params_str = serde_json::to_string_pretty(&overlay.request.parameters)
        .unwrap_or_else(|_| "{}".to_string());
    let params_preview: String = params_str.chars().take(100).collect();
    lines.push(Line::from(vec![
        Span::styled("Params: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(params_preview, Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));

    // Add selection buttons
    let yes_style = if overlay.selection == ApprovalSelection::Yes {
        Style::default().bg(Color::Green).fg(Color::Black)
    } else {
        Style::default().fg(Color::Green)
    };

    let no_style = if overlay.selection == ApprovalSelection::No {
        Style::default().bg(Color::Red).fg(Color::Black)
    } else {
        Style::default().fg(Color::Red)
    };

    let always_style = if overlay.selection == ApprovalSelection::Always {
        Style::default().bg(Color::Blue).fg(Color::Black)
    } else {
        Style::default().fg(Color::Blue)
    };

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(" [Y]es ", yes_style),
        Span::raw("  "),
        Span::styled(" [N]o ", no_style),
        Span::raw("  "),
        Span::styled(" [A]lways ", always_style),
    ]));

    let content = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, title_style)),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(content, overlay_area);
}

