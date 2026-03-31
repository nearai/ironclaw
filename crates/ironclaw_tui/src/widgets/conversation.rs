//! Conversation widget: renders chat messages with basic markdown.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use unicode_width::UnicodeWidthStr;

use crate::render::{format_tokens, format_tool_duration, render_markdown, truncate, wrap_text};
use crate::theme::Theme;

use super::{AppState, MessageRole, ToolActivity, ToolStatus, TuiWidget};

/// ASCII art startup banner displayed when the conversation is empty.
const BANNER: &[&str] = &[
    r"  ___                    ____ _                 ",
    r" |_ _|_ __ ___  _ __   / ___| | __ ___      __ ",
    r"  | || '__/ _ \| '_ \ | |   | |/ _` \ \ /\ / / ",
    r"  | || | | (_) | | | || |___| | (_| |\ V  V /  ",
    r" |___|_|  \___/|_| |_| \____|_|\__,_| \_/\_/   ",
];

/// Tagline shown beneath the ASCII art banner.
const BANNER_TAGLINE: &str = "Secure AI Assistant";

pub struct ConversationWidget {
    theme: Theme,
}

impl ConversationWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for ConversationWidget {
    fn id(&self) -> &str {
        "conversation"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        let usable_width = (area.width as usize).saturating_sub(4);
        let mut all_lines: Vec<Line<'_>> = Vec::new();


        // Welcome block when the conversation is empty
        if state.messages.is_empty() {
            self.render_welcome_screen(state, usable_width, &mut all_lines);
        }

        for msg in &state.messages {
            let (prefix, style) = match msg.role {
                MessageRole::User => ("\u{25CF} ", self.theme.accent_style()),
                MessageRole::Assistant => ("", Style::default().fg(self.theme.fg.to_color())),
                MessageRole::System => ("\u{25CB} ", self.theme.dim_style()),
            };

            if msg.role == MessageRole::User {
                // Blank line before user messages (except first)
                if !all_lines.is_empty() {
                    all_lines.push(Line::from(""));
                }
                let time_str = msg.timestamp.format("%H:%M").to_string();
                let user_line = Line::from(vec![
                    Span::styled(prefix.to_string(), self.theme.accent_style()),
                    Span::styled(msg.content.clone(), self.theme.bold_style()),
                    Span::styled(format!("  {time_str}"), self.theme.dim_style()),
                ]);
                all_lines.push(user_line);
                all_lines.push(Line::from(""));
            } else if msg.role == MessageRole::Assistant {
                // Separator with label before assistant response
                let turn_label = " ironclaw ";
                let sep_left_len = 2usize;
                let sep_right_len = usable_width
                    .min(60)
                    .saturating_sub(sep_left_len + turn_label.len());
                let sep_left = "\u{2500}".repeat(sep_left_len);
                let sep_right = "\u{2500}".repeat(sep_right_len);
                all_lines.push(Line::from(vec![
                    Span::styled(format!("  {sep_left}"), self.theme.dim_style()),
                    Span::styled(turn_label, self.theme.accent_style()),
                    Span::styled(sep_right, self.theme.dim_style()),
                ]));

                let wrapped =
                    render_markdown(&msg.content, usable_width.saturating_sub(2), &self.theme);
                for line in wrapped {
                    let mut padded = vec![Span::raw("  ".to_string())];
                    padded.extend(
                        line.spans
                            .into_iter()
                            .map(|s| Span::styled(s.content.to_string(), s.style)),
                    );
                    all_lines.push(Line::from(padded));
                }

                // Per-turn cost summary
                if let Some(ref cost) = msg.cost_summary {
                    let cost_line = format!(
                        "  \u{25CB} {}in + {}out  {}",
                        format_tokens(cost.input_tokens),
                        format_tokens(cost.output_tokens),
                        cost.cost_usd,
                    );
                    all_lines
                        .push(Line::from(Span::styled(cost_line, self.theme.dim_style())));
                }

                all_lines.push(Line::from(""));
            } else {
                let wrapped = wrap_text(&msg.content, usable_width, style);
                all_lines.extend(wrapped);
            }
        }

        // Inline tool calls (current turn only: tools started after last assistant message)
        let last_assistant_ts = state
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant)
            .map(|m| m.timestamp);

        let turn_recent: Vec<&ToolActivity> = state
            .recent_tools
            .iter()
            .filter(|t| match last_assistant_ts {
                Some(ts) => t.started_at > ts,
                None => true,
            })
            .collect();

        if !turn_recent.is_empty() || !state.active_tools.is_empty() {
            all_lines.push(Line::from(""));
            for tool in &turn_recent {
                all_lines.push(self.render_tool_line(tool, usable_width, false));
                // Tool output preview line
                if let Some(ref preview) = tool.result_preview {
                    let preview_max = usable_width.saturating_sub(8);
                    let first_line = preview.lines().next().unwrap_or("");
                    if !first_line.is_empty() {
                        all_lines.push(Line::from(vec![
                            Span::styled("  \u{250A}   ".to_string(), self.theme.dim_style()),
                            Span::styled("\u{2192} ".to_string(), self.theme.dim_style()),
                            Span::styled(
                                truncate(first_line, preview_max),
                                self.theme.dim_style(),
                            ),
                        ]));
                    }
                }
            }
            for tool in &state.active_tools {
                all_lines.push(self.render_tool_line(tool, usable_width, true));
                // Tool output preview line for active tools
                if let Some(ref preview) = tool.result_preview {
                    let preview_max = usable_width.saturating_sub(8);
                    let first_line = preview.lines().next().unwrap_or("");
                    if !first_line.is_empty() {
                        all_lines.push(Line::from(vec![
                            Span::styled("  \u{250A}   ".to_string(), self.theme.dim_style()),
                            Span::styled("\u{2192} ".to_string(), self.theme.dim_style()),
                            Span::styled(
                                truncate(first_line, preview_max),
                                self.theme.dim_style(),
                            ),
                        ]));
                    }
                }
            }
        }

        // Show thinking indicator if active
        const SPINNER: &[&str] = &["\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280F}"];

        if !state.status_text.is_empty() && !state.is_streaming {
            let frame = SPINNER[state.spinner_frame % SPINNER.len()];
            all_lines.push(Line::from(vec![
                Span::styled(format!("  {frame} "), self.theme.accent_style()),
                Span::styled(state.status_text.clone(), self.theme.dim_style()),
            ]));
        }

        // Show streaming dots indicator
        if state.is_streaming {
            let dots = match state.spinner_frame % 4 {
                0 => "\u{00B7}",
                1 => "\u{00B7}\u{00B7}",
                2 => "\u{00B7}\u{00B7}\u{00B7}",
                _ => "",
            };
            all_lines.push(Line::from(Span::styled(
                format!("  {dots}"),
                self.theme.accent_style(),
            )));
        }

        // Render follow-up suggestions when not streaming
        if !state.suggestions.is_empty() && !state.is_streaming {
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(Span::styled(
                "  Suggestions:".to_string(),
                self.theme.dim_style(),
            )));
            for (i, suggestion) in state.suggestions.iter().take(3).enumerate() {
                all_lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", i + 1), self.theme.accent_style()),
                    Span::styled(
                        truncate(suggestion, usable_width.saturating_sub(6)),
                        self.theme.dim_style(),
                    ),
                ]));
            }
        }

        // Search highlighting: replace spans that contain the query with
        // highlighted versions (black text on yellow background).
        if state.search.active && !state.search.query.is_empty() {
            let highlight_style = Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(ratatui::style::Color::Yellow);
            let query_lower = state.search.query.to_lowercase();

            all_lines = all_lines
                .into_iter()
                .map(|line| {
                    let mut new_spans: Vec<Span<'_>> = Vec::new();

                    for span in line.spans {
                        let text = span.content.to_string();
                        let text_lower = text.to_lowercase();

                        if text_lower.contains(&query_lower) {
                            let mut remaining = text.as_str();
                            while !remaining.is_empty() {
                                let lower_remaining = remaining.to_lowercase();
                                if let Some(pos) = lower_remaining.find(&query_lower) {
                                    if pos > 0 {
                                        new_spans.push(Span::styled(
                                            remaining[..pos].to_string(),
                                            span.style,
                                        ));
                                    }
                                    let match_end = pos + query_lower.len();
                                    new_spans.push(Span::styled(
                                        remaining[pos..match_end].to_string(),
                                        highlight_style,
                                    ));
                                    remaining = &remaining[match_end..];
                                } else {
                                    new_spans.push(Span::styled(
                                        remaining.to_string(),
                                        span.style,
                                    ));
                                    break;
                                }
                            }
                        } else {
                            new_spans.push(Span::styled(text, span.style));
                        }
                    }

                    Line::from(new_spans)
                })
                .collect();
        }

        // Compute visible window (scroll from bottom)
        let visible_height = area.height as usize;
        let total_lines = all_lines.len();
        let scroll = state.scroll_offset as usize;
        let start = total_lines.saturating_sub(visible_height + scroll);
        let end = total_lines.saturating_sub(scroll).min(total_lines);

        let mut visible: Vec<Line<'_>> = all_lines
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();

        // Insert search bar at top of visible area when search is active
        if state.search.active {
            let match_info = format!(
                "  {}/{}",
                if state.search.match_count > 0 {
                    state.search.current_match + 1
                } else {
                    0
                },
                state.search.match_count
            );
            let search_line = Line::from(vec![
                Span::styled(" / ", self.theme.accent_style()),
                Span::styled(state.search.query.clone(), self.theme.bold_style()),
                Span::styled(match_info, self.theme.dim_style()),
            ]);
            visible.insert(0, search_line);
            // Remove the last line to keep the total count consistent
            if visible.len() > visible_height {
                visible.pop();
            }
        }

        let paragraph = ratatui::widgets::Paragraph::new(visible);
        paragraph.render(area, buf);
    }
}

impl ConversationWidget {
    /// Return a category-specific icon and style for the given tool name.
    ///
    /// Categories are detected by simple substring matching on the tool name:
    /// - Shell/Bash (`shell`, `bash`, `exec`) -> `$` in warning/yellow
    /// - File (`file`, `read`, `write`, `edit`) -> `\u{270E}` in success/green
    /// - Web/HTTP (`http`, `web`, `fetch`) -> `\u{25CE}` in cyan
    /// - Memory (`memory`, `search`) -> `\u{25C8}` in magenta
    /// - Default -> `$` in dim style
    fn tool_category_icon(&self, tool_name: &str) -> (&'static str, Style) {
        let name = tool_name.to_lowercase();

        if name.contains("shell") || name.contains("bash") || name.contains("exec") {
            ("$", self.theme.warning_style())
        } else if name.contains("file")
            || name.contains("read")
            || name.contains("write")
            || name.contains("edit")
        {
            ("\u{270E}", self.theme.success_style()) // ✎
        } else if name.contains("http") || name.contains("web") || name.contains("fetch") {
            ("\u{25CE}", Style::default().fg(Color::Cyan)) // ◎
        } else if name.contains("memory") || name.contains("search") {
            ("\u{25C8}", Style::default().fg(Color::Magenta)) // ◈
        } else {
            ("$", self.theme.dim_style())
        }
    }

    /// Render a single tool call line in the Claude Code inline style.
    ///
    /// Format: `  \u{250A} icon category_icon  command_text...             1.3s`
    fn render_tool_line(
        &self,
        tool: &ToolActivity,
        usable_width: usize,
        is_active: bool,
    ) -> Line<'static> {
        let (icon, icon_style) = if is_active {
            ("\u{25CB}", self.theme.accent_style()) // ○ running
        } else {
            match tool.status {
                ToolStatus::Success => ("\u{25CF}", self.theme.success_style()), // ● green
                ToolStatus::Failed => ("\u{2717}", self.theme.error_style()),    // ✗ red
                ToolStatus::Running => ("\u{25CB}", self.theme.accent_style()),  // ○ accent
            }
        };

        // Duration text
        let duration_text = if is_active {
            let elapsed = chrono::Utc::now()
                .signed_duration_since(tool.started_at)
                .num_milliseconds()
                .unsigned_abs();
            format_tool_duration(elapsed)
        } else {
            tool.duration_ms
                .map(format_tool_duration)
                .unwrap_or_default()
        };

        // Determine category icon and style from the tool name
        let (cat_icon, cat_style) = self.tool_category_icon(&tool.name);

        // Build the command description: "cat_icon  detail" or "cat_icon  tool_name"
        let cmd_text = match &tool.detail {
            Some(d) => format!("{cat_icon}  {d}"),
            None => format!("{cat_icon}  {}", tool.name),
        };

        // Layout: "  \u{250A} icon  cmd...  duration"
        //          ^2  ^2    ^cmd    ^gap ^duration
        let prefix = format!("  \u{250A} {icon} ");
        let prefix_width = UnicodeWidthStr::width(prefix.as_str());
        let duration_width = UnicodeWidthStr::width(duration_text.as_str());
        let available_for_cmd =
            usable_width.saturating_sub(prefix_width + duration_width + 2); // 2 for gap

        let cmd_truncated = truncate(&cmd_text, available_for_cmd);
        let cmd_width = UnicodeWidthStr::width(cmd_truncated.as_str());

        // Pad between command and duration
        let gap = usable_width
            .saturating_sub(prefix_width + cmd_width + duration_width)
            .max(1);
        let padding = " ".repeat(gap);

        // Split cmd_truncated into the category icon part and the rest
        // so we can apply the category style to just the icon.
        let (styled_icon_part, rest_part) = if cmd_truncated.len() >= cat_icon.len()
            && cmd_truncated.starts_with(cat_icon)
        {
            (
                cmd_truncated[..cat_icon.len()].to_string(),
                cmd_truncated[cat_icon.len()..].to_string(),
            )
        } else {
            // Truncation cut into the icon; just dim everything
            (String::new(), cmd_truncated)
        };

        Line::from(vec![
            Span::styled("  \u{250A} ".to_string(), self.theme.dim_style()),
            Span::styled(format!("{icon} "), icon_style),
            Span::styled(styled_icon_part, cat_style),
            Span::styled(rest_part, self.theme.dim_style()),
            Span::raw(padding),
            Span::styled(duration_text, self.theme.dim_style()),
        ])
    }

    /// Render the Hermes-style welcome screen with two columns:
    /// left = ASCII art + model info, right = tools + skills.
    fn render_welcome_screen(
        &self,
        state: &AppState,
        _usable_width: usize,
        all_lines: &mut Vec<Line<'_>>,
    ) {
        let has_tools = !state.welcome_tools.is_empty();
        let has_skills = !state.welcome_skills.is_empty();

        // If no tools/skills data, fall back to simple centered layout
        if !has_tools && !has_skills {
            self.render_welcome_simple(state, all_lines);
            return;
        }

        // Build left-column lines (banner + metadata)
        let mut left_lines: Vec<Line<'_>> = Vec::new();

        // ASCII banner
        for banner_line in BANNER {
            left_lines.push(Line::from(Span::styled(
                (*banner_line).to_string(),
                self.theme.accent_style(),
            )));
        }
        left_lines.push(Line::from(Span::styled(
            format!("  {BANNER_TAGLINE}"),
            self.theme.dim_style(),
        )));

        // Padding between banner and metadata
        left_lines.push(Line::from(""));
        left_lines.push(Line::from(""));

        // Left column width: widest banner line + some padding
        let left_col_width = BANNER
            .iter()
            .map(|l| UnicodeWidthStr::width(*l))
            .max()
            .unwrap_or(40)
            + 4;
        // Max text width inside the left column (minus the 2-char indent)
        let left_text_max = left_col_width.saturating_sub(2);

        // Model + version
        let model_text = truncate(&state.model, left_text_max);
        left_lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(model_text, self.theme.accent_style()),
        ]));

        // Workspace path (truncated to fit left column)
        if !state.workspace_path.is_empty() {
            let path_text = truncate(&state.workspace_path, left_text_max);
            left_lines.push(Line::from(vec![
                Span::styled("  ".to_string(), Style::default()),
                Span::styled(path_text, self.theme.dim_style()),
            ]));
        }

        // Session ID
        let session_id = state
            .session_start
            .format("%Y%m%d_%H%M%S")
            .to_string();
        left_lines.push(Line::from(vec![
            Span::styled("  Session: ".to_string(), self.theme.dim_style()),
            Span::styled(session_id, self.theme.dim_style()),
        ]));

        // Build right-column lines (tools + skills)
        let mut right_lines: Vec<Line<'_>> = Vec::new();

        // Available Tools heading
        if has_tools {
            right_lines.push(Line::from(Span::styled(
                "Available Tools".to_string(),
                self.theme.bold_style(),
            )));

            let max_display = 12;
            let tool_count = state.welcome_tools.len();
            for cat in state.welcome_tools.iter().take(max_display) {
                right_lines
                    .push(Self::format_category_line(&cat.name, &cat.tools, &self.theme, true));
            }
            if tool_count > max_display {
                right_lines.push(Line::from(Span::styled(
                    format!("(and {} more toolsets...)", tool_count - max_display),
                    self.theme.dim_style(),
                )));
            }
        }

        // Blank separator
        if has_tools && has_skills {
            right_lines.push(Line::from(""));
        }

        // Available Skills heading
        if has_skills {
            right_lines.push(Line::from(Span::styled(
                "Available Skills".to_string(),
                self.theme.bold_style(),
            )));

            let max_display = 20;
            let skill_count = state.welcome_skills.len();
            for cat in state.welcome_skills.iter().take(max_display) {
                right_lines.push(Self::format_category_line(
                    &cat.name,
                    &cat.skills,
                    &self.theme,
                    false,
                ));
            }
            if skill_count > max_display {
                right_lines.push(Line::from(Span::styled(
                    format!("(and {} more...)", skill_count - max_display),
                    self.theme.dim_style(),
                )));
            }
        }

        // Footer summary
        let total_tools: usize = state.welcome_tools.iter().map(|c| c.tools.len()).sum();
        let total_skills: usize = state.welcome_skills.iter().map(|c| c.skills.len()).sum();
        right_lines.push(Line::from(""));
        right_lines.push(Line::from(vec![
            Span::styled(
                format!("{total_tools} tools"),
                self.theme.accent_style(),
            ),
            Span::styled("  \u{00B7}  ".to_string(), self.theme.dim_style()),
            Span::styled(
                format!("{total_skills} skills"),
                self.theme.accent_style(),
            ),
            Span::styled("  \u{00B7}  ".to_string(), self.theme.dim_style()),
            Span::styled("/help for commands".to_string(), self.theme.dim_style()),
        ]));

        // Compose two columns side-by-side
        let total_rows = left_lines.len().max(right_lines.len());

        for row in 0..total_rows {
            let left = left_lines.get(row);
            let right = right_lines.get(row);

            match (left, right) {
                (Some(l), Some(r)) => {
                    // Compute visual width of left line
                    let left_visual: usize = l
                        .spans
                        .iter()
                        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                        .sum();
                    let padding_needed = left_col_width.saturating_sub(left_visual);

                    let mut spans: Vec<Span<'_>> = Vec::new();
                    for s in &l.spans {
                        spans.push(Span::styled(s.content.to_string(), s.style));
                    }
                    spans.push(Span::raw(" ".repeat(padding_needed)));
                    for s in &r.spans {
                        spans.push(Span::styled(s.content.to_string(), s.style));
                    }
                    all_lines.push(Line::from(spans));
                }
                (Some(l), None) => {
                    let spans: Vec<Span<'_>> = l
                        .spans
                        .iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style))
                        .collect();
                    all_lines.push(Line::from(spans));
                }
                (None, Some(r)) => {
                    let mut spans = vec![Span::raw(" ".repeat(left_col_width))];
                    for s in &r.spans {
                        spans.push(Span::styled(s.content.to_string(), s.style));
                    }
                    all_lines.push(Line::from(spans));
                }
                (None, None) => {}
            }
        }

        // Trailing blank line
        all_lines.push(Line::from(""));
    }

    /// Simple welcome screen (no tools/skills data).
    fn render_welcome_simple(&self, state: &AppState, all_lines: &mut Vec<Line<'_>>) {
        for banner_line in BANNER {
            all_lines.push(Line::from(Span::styled(
                (*banner_line).to_string(),
                self.theme.accent_style(),
            )));
        }
        all_lines.push(Line::from(Span::styled(
            format!("  {BANNER_TAGLINE}"),
            self.theme.dim_style(),
        )));
        all_lines.push(Line::from(""));

        let context_label = format!("{} context", format_tokens(state.context_window));
        all_lines.push(Line::from(vec![
            Span::styled("  IronClaw".to_string(), self.theme.accent_style()),
            Span::styled(format!(" v{}", state.version), self.theme.accent_style()),
            Span::styled("  \u{00B7}  ".to_string(), self.theme.dim_style()),
            Span::styled(state.model.clone(), self.theme.dim_style()),
            Span::styled("  \u{00B7}  ".to_string(), self.theme.dim_style()),
            Span::styled(context_label, self.theme.dim_style()),
        ]));

        let time_str = state.session_start.format("%H:%M UTC").to_string();
        all_lines.push(Line::from(Span::styled(
            format!("  Session started {time_str}"),
            self.theme.dim_style(),
        )));
        all_lines.push(Line::from(""));
        all_lines.push(Line::from(vec![
            Span::styled(
                "  What can I help you with?".to_string(),
                self.theme.bold_style(),
            ),
            Span::styled("  /help for commands".to_string(), self.theme.dim_style()),
        ]));
    }

    /// Format a "category: item1, item2, item3, ..." line.
    fn format_category_line<'a>(
        name: &str,
        items: &[String],
        theme: &Theme,
        is_tool: bool,
    ) -> Line<'a> {
        let label_style = if is_tool {
            theme.warning_style()
        } else {
            theme.accent_style()
        };

        let mut items_str = items.join(", ");
        // Truncate if too long
        if items_str.len() > 60 {
            items_str.truncate(57);
            items_str.push_str("...");
        }

        Line::from(vec![
            Span::styled(format!("{name}: "), label_style),
            Span::styled(items_str, theme.dim_style()),
        ])
    }

    /// Handle scroll up/down. Returns true if scrolling occurred.
    pub fn scroll(&self, state: &mut AppState, delta: i16) {
        if delta < 0 {
            state.scroll_offset = state.scroll_offset.saturating_add(delta.unsigned_abs());
        } else {
            state.scroll_offset = state.scroll_offset.saturating_sub(delta as u16);
        }
    }
}
