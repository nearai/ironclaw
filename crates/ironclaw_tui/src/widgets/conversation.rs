//! Conversation widget: renders chat messages with basic markdown.

use std::sync::RwLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget};

use crate::layout::TuiSlot;
use unicode_width::UnicodeWidthStr;

use crate::render::{
    collapse_preview, format_tokens, format_tool_duration, render_markdown, truncate, wrap_text,
};
use crate::theme::Theme;
use crate::widgets::codeblock;
use crate::widgets::plan;

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

/// Number of characters revealed per tick frame during the boot animation.
const REVEAL_CHARS_PER_FRAME: u16 = 2;

/// Delay (in frames) between each banner line starting its reveal.
const REVEAL_LINE_STAGGER: u16 = 8;

/// Reveal a banner line with typewriter animation.
/// Returns the visible portion of `text` based on the current frame and line index.
fn reveal_text(text: &str, frame: u16, line_idx: u16) -> String {
    let stagger_offset = line_idx.saturating_mul(REVEAL_LINE_STAGGER);
    let effective = frame.saturating_sub(stagger_offset);
    let visible_chars = (effective as usize).saturating_mul(REVEAL_CHARS_PER_FRAME as usize);
    if visible_chars >= text.len() {
        text.to_string()
    } else {
        let mut result: String = text.chars().take(visible_chars).collect();
        // Pad with spaces to maintain layout width
        let remaining = text.chars().count().saturating_sub(visible_chars);
        result.extend(std::iter::repeat_n(' ', remaining));
        result
    }
}

#[derive(Default)]
struct ConversationRenderCache {
    usable_width: usize,
    messages: Vec<CachedRenderedMessage>,
    /// Total content lines computed during last render (used for scroll clamping).
    total_lines: usize,
    /// Visible height during last render (used for scroll clamping).
    visible_height: usize,
    /// Maps tool blocks to their line ranges in `all_lines`.
    /// Each entry is (start_idx, end_idx_exclusive, recent_tools_index).
    tool_regions: Vec<(usize, usize, usize)>,
    /// Index into `all_lines` for the first visible row on screen.
    visible_start: usize,
    /// Whether the first visible row is an overlay (search bar / scroll indicator).
    has_top_overlay: bool,
    /// Top Y coordinate of the conversation area on screen.
    area_y: u16,
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

        let lines_before_messages = all_lines.len();
        self.append_cached_message_lines(state, usable_width, &mut all_lines);

        // Inline tool calls (current turn only: tools started after last user message).
        // Using the user message timestamp rather than the assistant message
        // timestamp is critical for the v2 engine where all tool events
        // arrive before the response message.
        let last_user_ts = state
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.timestamp);
        let turn_recent: Vec<(usize, &ToolActivity)> = state
            .recent_tools
            .iter()
            .enumerate()
            .filter(|(_, t)| match last_user_ts {
                Some(ts) => t.started_at >= ts,
                None => true,
            })
            .collect();

        // Build tool lines in a temporary vec so we can splice them into
        // the correct position (after the last user message, before any
        // assistant responses from the current turn).
        let mut tool_lines: Vec<Line<'static>> = Vec::new();
        let mut tool_regions: Vec<(usize, usize, usize)> = Vec::new();

        if !turn_recent.is_empty() || !state.active_tools.is_empty() {
            tool_lines.push(Line::from(""));
            for (rt_idx, tool) in &turn_recent {
                let block_start = tool_lines.len();
                if tool.result_preview.is_some() {
                    let block_lines = codeblock::render_tool_block(tool, usable_width, &self.theme);
                    tool_lines.extend(block_lines);
                    tool_lines.push(Line::from(""));
                } else {
                    tool_lines.push(self.render_tool_line(tool, usable_width, false));
                }
                tool_regions.push((block_start, tool_lines.len(), *rt_idx));
            }
            for tool in &state.active_tools {
                tool_lines.push(self.render_tool_line(tool, usable_width, true));
                if let Some(ref preview) = tool.result_preview {
                    let preview_max = usable_width.saturating_sub(8);
                    let collapsed = collapse_preview(preview, preview_max);
                    if !collapsed.is_empty() {
                        tool_lines.push(Line::from(vec![
                            Span::styled("  \u{250A}   ".to_string(), self.theme.dim_style()),
                            Span::styled("\u{2192} ".to_string(), self.theme.dim_style()),
                            Span::styled(collapsed, self.theme.dim_style()),
                        ]));
                    }
                }
            }
        }

        // Insert tool lines BEFORE the current turn's assistant messages.
        // Tools execute before the response, so they should render above it.
        let tool_insert_pos = if !tool_lines.is_empty() {
            let last_user_idx =
                state.messages.iter().rposition(|m| m.role == MessageRole::User);
            if let Some(ui) = last_user_idx {
                let cache = self
                    .render_cache
                    .read()
                    .unwrap_or_else(|p| p.into_inner());
                let mut pos = lines_before_messages;
                for i in 0..=ui {
                    if let Some(entry) = cache.messages.get(i) {
                        pos += entry.lines.len();
                    }
                }
                drop(cache);
                pos
            } else {
                all_lines.len()
            }
        } else {
            all_lines.len()
        };

        // Offset tool_regions by the actual insert position in all_lines
        for region in &mut tool_regions {
            region.0 += tool_insert_pos;
            region.1 += tool_insert_pos;
        }

        // Splice tool lines into the correct position
        all_lines.splice(tool_insert_pos..tool_insert_pos, tool_lines);

        // Render active plan inline
        if let Some(ref plan_state) = state.plan_state {
            all_lines.push(Line::from(""));
            let plan_lines =
                plan::render_plan_block(plan_state, usable_width, state.tick_count, &self.theme);
            all_lines.extend(plan_lines);
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
        // (tool_regions and visible_start are updated below after scroll computation)
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
        let has_top_overlay;
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
            has_top_overlay = true;
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
            has_top_overlay = true;
        } else {
            has_top_overlay = false;
        }

        // Store tool click regions for mouse hit testing
        if let Ok(mut cache) = self.render_cache.write() {
            cache.tool_regions = tool_regions;
            cache.visible_start = start;
            cache.has_top_overlay = has_top_overlay;
            cache.area_y = area.y;
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
                lines.push(Line::from(vec![
                    Span::styled("\u{25CF} ".to_string(), self.theme.accent_style()),
                    Span::styled(message.content.clone(), self.theme.bold_style()),
                    Span::styled(format!("  {time_str}"), self.theme.dim_style()),
                ]));
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

    /// Render a single tool call line with a readable name tag.
    ///
    /// Format: `  ┊ ○ Read  src/main.rs                  1.3s`
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

        // Tool display name tag: "Read", "Bash", "Glob", etc.
        let display_name = codeblock::format_display_name(&tool.name);
        let is_read = display_name == "Read";
        let tag_style = if is_read {
            self.theme.tool_read_dot_style()
        } else {
            self.theme.tool_action_dot_style()
        };

        // Detail text (path, command, query)
        let detail_text = tool.detail.as_deref().unwrap_or(&tool.name);

        // Layout: "  ┊ ○ Tag  detail...  duration"
        let prefix_str = format!("  \u{250A} {icon} {display_name} ");
        let prefix_width = UnicodeWidthStr::width(prefix_str.as_str());
        let duration_width = UnicodeWidthStr::width(duration_text.as_str());
        let available_for_detail = usable_width.saturating_sub(prefix_width + duration_width + 2);

        let detail_truncated = truncate(detail_text, available_for_detail);
        let detail_width = UnicodeWidthStr::width(detail_truncated.as_str());

        let gap = usable_width
            .saturating_sub(prefix_width + detail_width + duration_width)
            .max(1);
        let padding = " ".repeat(gap);

        Line::from(vec![
            Span::styled("  \u{250A} ".to_string(), self.theme.dim_style()),
            Span::styled(format!("{icon} "), icon_style),
            Span::styled(
                format!("{display_name} "),
                Style::default()
                    .fg(tag_style.fg.unwrap_or(Color::White))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(detail_truncated, self.theme.dim_style()),
            Span::raw(padding),
            Span::styled(duration_text, self.theme.dim_style()),
        ])
    }

    /// Render the welcome screen: centered banner + system info + compact capabilities.
    fn render_welcome_screen(
        &self,
        state: &AppState,
        usable_width: usize,
        all_lines: &mut Vec<Line<'static>>,
    ) {
        let frame = state.welcome_reveal_frame;

        // ── Animated ASCII banner ────────────────────────
        all_lines.push(Line::from(""));
        for (i, banner_line) in BANNER.iter().enumerate() {
            let text = reveal_text(banner_line, frame, i as u16);
            all_lines.push(Line::from(Span::styled(text, self.theme.accent_style())));
        }
        let tagline = reveal_text(&format!("  {BANNER_TAGLINE}"), frame, BANNER.len() as u16);
        all_lines.push(Line::from(Span::styled(tagline, self.theme.dim_style())));
        all_lines.push(Line::from(""));

        // ── System info line ─────────────────────────────
        let ctx_label = format!("{}K ctx", state.context_window / 1000);
        let mut info_spans = vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(state.model.clone(), self.theme.accent_style()),
            Span::styled("  \u{00B7}  ".to_string(), self.theme.dim_style()),
            Span::styled(ctx_label, self.theme.dim_style()),
        ];
        if !state.workspace_path.is_empty() {
            let max_path = usable_width.saturating_sub(40).min(40);
            let path = truncate(&state.workspace_path, max_path);
            info_spans.push(Span::styled(
                "  \u{00B7}  ".to_string(),
                self.theme.dim_style(),
            ));
            info_spans.push(Span::styled(path, self.theme.dim_style()));
        }
        all_lines.push(Line::from(info_spans));

        // ── Identity & memory ────────────────────────────
        if state.memory_count > 0 || !state.identity_files.is_empty() {
            let mut meta_spans = vec![Span::styled("  ".to_string(), Style::default())];
            if state.memory_count > 0 {
                meta_spans.push(Span::styled(
                    format!("\u{25C8} {} memories", state.memory_count),
                    self.theme.dim_style(),
                ));
            }
            if !state.identity_files.is_empty() {
                if state.memory_count > 0 {
                    meta_spans.push(Span::styled(
                        "  \u{00B7}  ".to_string(),
                        self.theme.dim_style(),
                    ));
                }
                meta_spans.push(Span::styled(
                    format!("\u{25CB} {}", state.identity_files.join(", ")),
                    self.theme.dim_style(),
                ));
            }
            all_lines.push(Line::from(meta_spans));
        }

        // ── Separator ────────────────────────────────────
        let sep_width = usable_width.min(60);
        all_lines.push(Line::from(""));
        all_lines.push(Line::from(Span::styled(
            format!("  {}", "\u{2500}".repeat(sep_width)),
            self.theme.dim_style(),
        )));
        all_lines.push(Line::from(""));

        // ── Capabilities summary (compact) ───────────────
        let has_tools = !state.welcome_tools.is_empty();
        let has_skills = !state.welcome_skills.is_empty();

        if has_tools {
            let total_tools: usize = state.welcome_tools.iter().map(|c| c.tools.len()).sum();
            let cat_names: Vec<String> = state
                .welcome_tools
                .iter()
                .take(8)
                .map(|c| c.name.clone())
                .collect();
            let more = if state.welcome_tools.len() > 8 {
                format!(" +{}", state.welcome_tools.len() - 8)
            } else {
                String::new()
            };
            all_lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{25B8} {total_tools} tools"),
                    self.theme.accent_style(),
                ),
                Span::styled(
                    format!("  {}{more}", cat_names.join(" \u{00B7} ")),
                    self.theme.dim_style(),
                ),
            ]));
        }

        if has_skills {
            let total_skills: usize = state.welcome_skills.iter().map(|c| c.skills.len()).sum();
            let cat_names: Vec<String> = state
                .welcome_skills
                .iter()
                .take(8)
                .map(|c| c.name.clone())
                .collect();
            let more = if state.welcome_skills.len() > 8 {
                format!(" +{}", state.welcome_skills.len() - 8)
            } else {
                String::new()
            };
            all_lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{25B8} {total_skills} skills"),
                    self.theme.accent_style(),
                ),
                Span::styled(
                    format!("  {}{more}", cat_names.join(" \u{00B7} ")),
                    self.theme.dim_style(),
                ),
            ]));
        }

        // ── Prompt ───────────────────────────────────────
        all_lines.push(Line::from(""));
        all_lines.push(Line::from(vec![
            Span::styled(
                "  What can I help you with?".to_string(),
                self.theme.bold_style(),
            ),
            Span::styled("  /help for commands".to_string(), self.theme.dim_style()),
        ]));
    }

    /// Returns the index into `state.recent_tools` if the given screen row
    /// falls within a rendered tool block. Used for click-to-expand.
    pub fn tool_index_at_row(&self, row: u16) -> Option<usize> {
        let cache = self.render_cache.read().ok()?;

        if row < cache.area_y {
            return None;
        }

        let offset = (row - cache.area_y) as usize;
        if cache.has_top_overlay && offset == 0 {
            return None;
        }

        let all_lines_idx = if cache.has_top_overlay {
            cache.visible_start + offset - 1
        } else {
            cache.visible_start + offset
        };

        cache
            .tool_regions
            .iter()
            .find(|(s, e, _)| all_lines_idx >= *s && all_lines_idx < *e)
            .map(|(_, _, idx)| *idx)
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
