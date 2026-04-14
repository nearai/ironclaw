//! Conversation widget: renders chat messages with basic markdown.

use std::sync::RwLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget};

use crate::layout::TuiSlot;
use unicode_width::UnicodeWidthStr;

use crate::render::{
    collapse_preview, format_tokens, format_tool_duration, render_markdown, truncate, wrap_text,
};
use crate::theme::Theme;

use super::{AppState, ChatMessage, MessageRole, ToolActivity, ToolStatus, TuiWidget};

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

#[derive(Default)]
struct ConversationRenderCache {
    usable_width: usize,
    messages: Vec<CachedRenderedMessage>,
    /// Total content lines computed during last render (used for scroll clamping).
    total_lines: usize,
    /// Visible height during last render (used for scroll clamping).
    visible_height: usize,
}

struct CachedRenderedMessage {
    message: ChatMessage,
    is_first_message: bool,
    lines: Vec<Line<'static>>,
}

impl CachedRenderedMessage {
    fn matches(&self, message: &ChatMessage, is_first_message: bool) -> bool {
        self.is_first_message == is_first_message && self.message == *message
    }
}

pub struct ConversationWidget {
    theme: Theme,
    render_cache: RwLock<ConversationRenderCache>,
}

impl ConversationWidget {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            render_cache: RwLock::new(ConversationRenderCache::default()),
        }
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
        let mut all_lines: Vec<Line<'static>> = Vec::new();

        // Welcome block when the conversation is empty
        if state.messages.is_empty() {
            self.render_welcome_screen(state, usable_width, &mut all_lines);
        }

        self.append_cached_message_lines(state, usable_width, &mut all_lines);

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
                if let Some(ref preview) = tool.result_preview {
                    let preview_max = usable_width.saturating_sub(8);
                    let collapsed = collapse_preview(preview, preview_max);
                    if !collapsed.is_empty() {
                        all_lines.push(Line::from(vec![
                            Span::styled("  \u{250A}   ".to_string(), self.theme.dim_style()),
                            Span::styled("\u{2192} ".to_string(), self.theme.dim_style()),
                            Span::styled(collapsed, self.theme.dim_style()),
                        ]));
                    }
                }
            }
            for tool in &state.active_tools {
                all_lines.push(self.render_tool_line(tool, usable_width, true));
                if let Some(ref preview) = tool.result_preview {
                    let preview_max = usable_width.saturating_sub(8);
                    let collapsed = collapse_preview(preview, preview_max);
                    if !collapsed.is_empty() {
                        all_lines.push(Line::from(vec![
                            Span::styled("  \u{250A}   ".to_string(), self.theme.dim_style()),
                            Span::styled("\u{2192} ".to_string(), self.theme.dim_style()),
                            Span::styled(collapsed, self.theme.dim_style()),
                        ]));
                    }
                }
            }
        }

        // Show thinking indicator if active (tick interval = 33ms)
        const TICK_MS: u64 = 33;

        if !state.status_text.is_empty() && !state.is_streaming {
            let frame = state.spinner.frame(state.tick_count, TICK_MS);
            all_lines.push(Line::from(vec![
                Span::styled(format!("  {frame} "), self.theme.accent_style()),
                Span::styled(state.status_text.clone(), self.theme.dim_style()),
            ]));
        }

        // Show streaming dots indicator
        if state.is_streaming {
            let dots = match (state.tick_count / 4) % 4 {
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
                    let mut new_spans: Vec<Span<'static>> = Vec::new();

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
                                    new_spans.push(Span::styled(remaining.to_string(), span.style));
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

        // Store for scroll clamping in scroll()
        if let Ok(mut cache) = self.render_cache.write() {
            cache.total_lines = total_lines;
            cache.visible_height = visible_height;
        }

        // Clamp scroll offset to valid range
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = (state.scroll_offset as usize).min(max_scroll);

        let start = total_lines.saturating_sub(visible_height + scroll);
        let end = total_lines.saturating_sub(scroll).min(total_lines);

        let mut visible: Vec<Line<'static>> = all_lines
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
            if visible.len() > visible_height {
                visible.pop();
            }
        } else if scroll > 0 && start > 0 {
            // Scroll position indicator when not at bottom
            let indicator = format!("\u{2191} {start} more ");
            let indicator_line = Line::from(vec![
                Span::styled(
                    " ".repeat(area.width as usize - indicator.chars().count() - 1),
                    self.theme.dim_style(),
                ),
                Span::styled(indicator, self.theme.dim_style()),
            ]);
            visible.insert(0, indicator_line);
            if visible.len() > visible_height {
                visible.pop();
            }
        }

        // "↓ N more" indicator at bottom when scrolled up
        if scroll > 0 {
            let indicator = format!("\u{2193} {scroll} more \u{2193} End to return ");
            if let Some(last) = visible.last_mut() {
                let pad_len = (area.width as usize).saturating_sub(indicator.len() + 1);
                *last = Line::from(vec![
                    Span::styled(" ".repeat(pad_len), self.theme.dim_style()),
                    Span::styled(indicator, self.theme.accent_style()),
                ]);
            }
        }

        let paragraph = ratatui::widgets::Paragraph::new(visible);
        paragraph.render(area, buf);

        // Render scrollbar when content exceeds viewport
        if total_lines > visible_height {
            let position = total_lines.saturating_sub(visible_height + scroll);
            let mut scrollbar_state =
                ScrollbarState::new(total_lines.saturating_sub(visible_height)).position(position);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("\u{2502}"))
                .thumb_symbol("\u{2503}");
            scrollbar.render(area, buf, &mut scrollbar_state);
        }
    }
}

impl ConversationWidget {
    fn append_cached_message_lines(
        &self,
        state: &AppState,
        usable_width: usize,
        all_lines: &mut Vec<Line<'static>>,
    ) {
        let mut cache = match self.render_cache.write() {
            Ok(cache) => cache,
            Err(poisoned) => {
                tracing::debug!("conversation render cache lock poisoned; continuing");
                poisoned.into_inner()
            }
        };
        if cache.usable_width != usable_width {
            cache.usable_width = usable_width;
            cache.messages.clear();
        }

        cache.messages.truncate(state.messages.len());

        for (index, message) in state.messages.iter().enumerate() {
            let is_first_message = index == 0;
            let needs_refresh = match cache.messages.get(index) {
                Some(entry) => !entry.matches(message, is_first_message),
                None => true,
            };

            if needs_refresh {
                let rendered = CachedRenderedMessage {
                    message: message.clone(),
                    is_first_message,
                    lines: self.render_message_lines(message, usable_width, is_first_message),
                };

                if index < cache.messages.len() {
                    cache.messages[index] = rendered;
                } else {
                    cache.messages.push(rendered);
                }
            }

            if let Some(entry) = cache.messages.get(index) {
                all_lines.extend(entry.lines.iter().cloned());
            }
        }
    }

    fn render_message_lines(
        &self,
        message: &ChatMessage,
        usable_width: usize,
        is_first_message: bool,
    ) -> Vec<Line<'static>> {
        match message.role {
            MessageRole::User => {
                let mut lines = Vec::new();
                if !is_first_message {
                    lines.push(Line::from(""));
                }

                let time_str = message.timestamp.format("%H:%M").to_string();
                let mut content_lines = message.content.split('\n');
                let first_line = content_lines.next().unwrap_or("").to_string();
                lines.push(Line::from(vec![
                    Span::styled("\u{25CF} ".to_string(), self.theme.accent_style()),
                    Span::styled(first_line, self.theme.bold_style()),
                    Span::styled(format!("  {time_str}"), self.theme.dim_style()),
                ]));
                for extra in content_lines {
                    lines.push(Line::from(vec![
                        Span::raw("  ".to_string()),
                        Span::styled(extra.to_string(), self.theme.bold_style()),
                    ]));
                }
                lines.push(Line::from(""));
                lines
            }
            MessageRole::Assistant => {
                let time_str = message.timestamp.format("%H:%M").to_string();
                let turn_label = " ironclaw ";
                let time_label = format!(" {time_str} ");
                let sep_left_len = 2usize;
                let sep_right_len = usable_width
                    .min(60)
                    .saturating_sub(sep_left_len + turn_label.len() + time_label.len());
                let sep_left = "\u{2500}".repeat(sep_left_len);
                let sep_right = "\u{2500}".repeat(sep_right_len);
                let mut lines = vec![Line::from(vec![
                    Span::styled(format!("  {sep_left}"), self.theme.dim_style()),
                    Span::styled(turn_label, self.theme.accent_style()),
                    Span::styled(sep_right, self.theme.dim_style()),
                    Span::styled(time_label, self.theme.dim_style()),
                ])];

                for line in render_markdown(
                    &message.content,
                    usable_width.saturating_sub(2),
                    &self.theme,
                ) {
                    let mut padded = vec![Span::raw("  ".to_string())];
                    padded.extend(line.spans);
                    lines.push(Line::from(padded));
                }

                if let Some(ref cost) = message.cost_summary {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "  \u{25CB} {}in + {}out  {}",
                            format_tokens(cost.input_tokens),
                            format_tokens(cost.output_tokens),
                            cost.cost_usd,
                        ),
                        self.theme.dim_style(),
                    )));
                }

                lines.push(Line::from(""));
                lines
            }
            MessageRole::System => {
                let time_str = message.timestamp.format("%H:%M").to_string();
                let mut lines = Vec::new();
                for (index, line) in wrap_text(
                    &message.content,
                    usable_width.saturating_sub(8),
                    self.theme.dim_style(),
                )
                .into_iter()
                .enumerate()
                {
                    if index == 0 {
                        let mut spans = line.spans;
                        spans.push(Span::styled(
                            format!("  {time_str}"),
                            self.theme.dim_style(),
                        ));
                        lines.push(Line::from(spans));
                    } else {
                        lines.push(line);
                    }
                }
                lines
            }
        }
    }

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
        let available_for_cmd = usable_width.saturating_sub(prefix_width + duration_width + 2); // 2 for gap

        let cmd_truncated = truncate(&cmd_text, available_for_cmd);
        let cmd_width = UnicodeWidthStr::width(cmd_truncated.as_str());

        // Pad between command and duration
        let gap = usable_width
            .saturating_sub(prefix_width + cmd_width + duration_width)
            .max(1);
        let padding = " ".repeat(gap);

        // Split cmd_truncated into the category icon part and the rest
        // so we can apply the category style to just the icon.
        let (styled_icon_part, rest_part) =
            if cmd_truncated.len() >= cat_icon.len() && cmd_truncated.starts_with(cat_icon) {
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
        all_lines: &mut Vec<Line<'static>>,
    ) {
        let has_tools = !state.welcome_tools.is_empty();
        let has_skills = !state.welcome_skills.is_empty();

        // If no tools/skills data, fall back to simple centered layout
        if !has_tools && !has_skills {
            self.render_welcome_simple(state, all_lines);
            return;
        }

        // Build left-column lines (banner + metadata)
        let mut left_lines: Vec<Line<'static>> = Vec::new();

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

        // Context window
        let ctx_label = format!("{}K context", state.context_window / 1000);
        left_lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(ctx_label, self.theme.dim_style()),
        ]));

        // Session ID
        let session_id = state.session_start.format("%Y%m%d_%H%M%S").to_string();
        left_lines.push(Line::from(vec![
            Span::styled("  Session: ".to_string(), self.theme.dim_style()),
            Span::styled(session_id, self.theme.dim_style()),
        ]));

        // Memory / workspace stats
        if state.memory_count > 0 || !state.identity_files.is_empty() {
            left_lines.push(Line::from(""));

            if state.memory_count > 0 {
                left_lines.push(Line::from(vec![
                    Span::styled("  \u{25C8} ".to_string(), self.theme.accent_style()),
                    Span::styled(
                        format!("{} memories", state.memory_count),
                        self.theme.dim_style(),
                    ),
                ]));
            }

            if !state.identity_files.is_empty() {
                let files_str = state.identity_files.join(", ");
                let files_display = truncate(&files_str, left_text_max.saturating_sub(4));
                left_lines.push(Line::from(vec![
                    Span::styled("  \u{25CB} ".to_string(), self.theme.accent_style()),
                    Span::styled(files_display, self.theme.dim_style()),
                ]));
            }
        }

        // Build right-column lines (tools + skills)
        let mut right_lines: Vec<Line<'static>> = Vec::new();

        // Available Tools heading
        if has_tools {
            right_lines.push(Line::from(Span::styled(
                "Available Tools".to_string(),
                self.theme.bold_style(),
            )));

            let max_display = 12;
            let tool_count = state.welcome_tools.len();
            for cat in state.welcome_tools.iter().take(max_display) {
                right_lines.push(Self::format_category_line(
                    &cat.name,
                    &cat.tools,
                    &self.theme,
                    true,
                ));
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
        let mut footer_spans = vec![
            Span::styled(format!("{total_tools} tools"), self.theme.accent_style()),
            Span::styled("  \u{00B7}  ".to_string(), self.theme.dim_style()),
            Span::styled(format!("{total_skills} skills"), self.theme.accent_style()),
        ];
        if state.memory_count > 0 {
            footer_spans.push(Span::styled(
                "  \u{00B7}  ".to_string(),
                self.theme.dim_style(),
            ));
            footer_spans.push(Span::styled(
                format!("{} memories", state.memory_count),
                self.theme.accent_style(),
            ));
        }
        footer_spans.push(Span::styled(
            "  \u{00B7}  ".to_string(),
            self.theme.dim_style(),
        ));
        footer_spans.push(Span::styled(
            "/help for commands".to_string(),
            self.theme.dim_style(),
        ));
        right_lines.push(Line::from(footer_spans));

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

                    let mut spans: Vec<Span<'static>> = Vec::new();
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
                    let spans: Vec<Span<'static>> = l
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
    fn render_welcome_simple(&self, state: &AppState, all_lines: &mut Vec<Line<'static>>) {
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
    fn format_category_line(
        name: &str,
        items: &[String],
        theme: &Theme,
        is_tool: bool,
    ) -> Line<'static> {
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

    /// Handle scroll up/down with clamping and auto-follow management.
    pub fn scroll(&self, state: &mut AppState, delta: i16) {
        let max_scroll = {
            let cache = match self.render_cache.read() {
                Ok(c) => c,
                Err(p) => p.into_inner(),
            };
            cache.total_lines.saturating_sub(cache.visible_height) as u16
        };

        if delta < 0 {
            // Scrolling up
            state.scroll_offset = state
                .scroll_offset
                .saturating_add(delta.unsigned_abs())
                .min(max_scroll);
            state.pinned_to_bottom = false;
        } else {
            // Scrolling down
            state.scroll_offset = state.scroll_offset.saturating_sub(delta as u16);
            if state.scroll_offset == 0 {
                state.pinned_to_bottom = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn user_message(content: &str) -> ChatMessage {
        ChatMessage {
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: chrono::Utc::now(),
            cost_summary: None,
        }
    }

    fn span_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn user_message_with_newlines_renders_as_multiple_lines() {
        let widget = ConversationWidget::new(Theme::dark());
        let lines = widget.render_message_lines(&user_message("hi\nhello\ngoodbye"), 80, true);

        let body: Vec<String> = lines
            .iter()
            .map(span_text)
            .filter(|s| !s.trim().is_empty())
            .collect();

        assert!(
            body.iter().any(|l| l.contains("hi")),
            "missing first line: {body:?}"
        );
        assert!(
            body.iter().any(|l| l.contains("hello")),
            "missing second line: {body:?}"
        );
        assert!(
            body.iter().any(|l| l.contains("goodbye")),
            "missing third line: {body:?}"
        );
        // The three logical lines must each get their own ratatui Line.
        let hi_line = body.iter().position(|l| l.contains("hi")).unwrap();
        let hello_line = body.iter().position(|l| l.contains("hello")).unwrap();
        let goodbye_line = body.iter().position(|l| l.contains("goodbye")).unwrap();
        assert!(hi_line < hello_line && hello_line < goodbye_line);
    }

    #[test]
    fn single_line_user_message_still_renders() {
        let widget = ConversationWidget::new(Theme::dark());
        let lines = widget.render_message_lines(&user_message("just one line"), 80, true);
        let joined: String = lines.iter().map(span_text).collect::<Vec<_>>().join("|");
        assert!(joined.contains("just one line"));
    }
}
