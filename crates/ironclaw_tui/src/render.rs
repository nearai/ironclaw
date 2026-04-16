//! Rendering utilities for converting text to styled Ratatui spans.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

/// Convert a plain text string into wrapped `Line`s that fit within `max_width`.
pub fn wrap_text(text: &str, max_width: usize, style: Style) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![];
    }

    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        lines.extend(wrap_plain_line(raw_line, max_width, style));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

fn wrap_plain_line(raw_line: &str, max_width: usize, style: Style) -> Vec<Line<'static>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_whitespace = None;

    for ch in raw_line.chars() {
        let is_whitespace = ch.is_whitespace();
        if current_is_whitespace == Some(is_whitespace) || current.is_empty() {
            push_wrapped_char(&mut current, ch);
            current_is_whitespace = Some(is_whitespace);
            continue;
        }

        tokens.push(std::mem::take(&mut current));
        push_wrapped_char(&mut current, ch);
        current_is_whitespace = Some(is_whitespace);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;

    for token in tokens {
        let token_width = UnicodeWidthStr::width(token.as_str());
        if current_width + token_width <= max_width {
            current_line.push_str(&token);
            current_width += token_width;
            continue;
        }

        if !current_line.is_empty() {
            lines.push(Line::from(Span::styled(
                std::mem::take(&mut current_line),
                style,
            )));
            current_width = 0;
        }

        if token_width <= max_width {
            current_width = token_width;
            current_line = token;
            continue;
        }

        for ch in token.chars() {
            let rendered = render_wrapped_char(ch);
            let rendered_width = wrapped_char_width(ch);
            if current_width + rendered_width > max_width && !current_line.is_empty() {
                lines.push(Line::from(Span::styled(
                    std::mem::take(&mut current_line),
                    style,
                )));
                current_width = 0;
            }
            current_line.push_str(&rendered);
            current_width += rendered_width;
        }
    }

    if !current_line.is_empty() {
        lines.push(Line::from(Span::styled(current_line, style)));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

fn push_wrapped_char(target: &mut String, ch: char) {
    target.push_str(&render_wrapped_char(ch));
}

fn render_wrapped_char(ch: char) -> String {
    match ch {
        '\t' => "    ".to_string(),
        _ => ch.to_string(),
    }
}

fn wrapped_char_width(ch: char) -> usize {
    match ch {
        '\t' => 4,
        _ => UnicodeWidthChar::width(ch).unwrap_or(0),
    }
}

// ── Markdown rendering ────────────────────────────────────────────────

/// Which kind of list we're inside.
#[derive(Clone)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

/// Render CommonMark `text` into styled, word-wrapped `Line`s.
///
/// Headings, bold, italic, inline code, fenced code blocks, lists,
/// blockquotes, horizontal rules, and links are all rendered with
/// appropriate terminal styles via `theme`.
pub fn render_markdown(text: &str, max_width: usize, theme: &Theme) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![];
    }

    let opts = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(text, opts);

    let mut ctx = MdContext::new(theme);

    for event in parser {
        match event {
            // ── Block-level start ────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                if !ctx.first_block {
                    ctx.need_blank_line = true;
                }
                if ctx.need_blank_line {
                    ctx.lines.push(Line::from(""));
                    ctx.need_blank_line = false;
                }
                let heading_style = match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => theme.bold_accent_style(),
                    _ => theme.bold_style(),
                };
                ctx.style_stack.push(heading_style);
            }
            Event::End(TagEnd::Heading(_)) => {
                ctx.flush(max_width, theme);
                ctx.style_stack.pop();
                ctx.need_blank_line = true;
                ctx.first_block = false;
            }

            Event::Start(Tag::Paragraph) if ctx.need_blank_line && !ctx.first_block => {
                ctx.lines.push(Line::from(""));
                ctx.need_blank_line = false;
            }
            Event::End(TagEnd::Paragraph) => {
                ctx.flush(max_width, theme);
                ctx.need_blank_line = true;
                ctx.first_block = false;
            }

            Event::Start(Tag::BlockQuote(_)) => {
                if ctx.need_blank_line && !ctx.first_block {
                    ctx.lines.push(Line::from(""));
                    ctx.need_blank_line = false;
                }
                ctx.in_blockquote = true;
                ctx.style_stack.push(theme.dim_style());
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                ctx.flush(max_width, theme);
                ctx.in_blockquote = false;
                ctx.style_stack.pop();
                ctx.need_blank_line = true;
                ctx.first_block = false;
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                if ctx.need_blank_line && !ctx.first_block {
                    ctx.lines.push(Line::from(""));
                    ctx.need_blank_line = false;
                }
                // Language badge for fenced code blocks
                if let CodeBlockKind::Fenced(ref lang) = kind {
                    let lang_str = lang.split(',').next().unwrap_or("").trim();
                    if !lang_str.is_empty() {
                        ctx.lines.push(Line::from(Span::styled(
                            format!("[{lang_str}]"),
                            theme.accent_style().add_modifier(Modifier::BOLD),
                        )));
                    }
                }
                ctx.in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                ctx.in_code_block = false;
                ctx.need_blank_line = true;
                ctx.first_block = false;
            }

            Event::Start(Tag::List(start)) => {
                if ctx.need_blank_line && !ctx.first_block {
                    ctx.lines.push(Line::from(""));
                    ctx.need_blank_line = false;
                }
                match start {
                    Some(n) => ctx.list_stack.push(ListKind::Ordered(n)),
                    None => ctx.list_stack.push(ListKind::Unordered),
                }
            }
            Event::End(TagEnd::List(_)) => {
                ctx.list_stack.pop();
                ctx.need_blank_line = true;
                ctx.first_block = false;
            }

            Event::Start(Tag::Item) => {
                let depth = ctx.list_stack.len().saturating_sub(1);
                let base_indent = depth * 4;
                let prefix = match ctx.list_stack.last() {
                    Some(ListKind::Unordered) => {
                        format!("{}\u{2022} ", " ".repeat(base_indent + 2))
                    }
                    Some(ListKind::Ordered(n)) => {
                        format!("{}{}. ", " ".repeat(base_indent + 1), n)
                    }
                    None => String::new(),
                };
                let style = ctx.top_style();
                ctx.segments.push((prefix, style));
            }
            Event::End(TagEnd::Item) => {
                ctx.flush(max_width, theme);
                if let Some(ListKind::Ordered(n)) = ctx.list_stack.last_mut() {
                    *n += 1;
                }
                ctx.first_block = false;
            }

            // ── Inline formatting ────────────────────────────────
            Event::Start(Tag::Strong) => {
                let s = ctx.top_style().add_modifier(Modifier::BOLD);
                ctx.style_stack.push(s);
            }
            Event::End(TagEnd::Strong) => {
                ctx.style_stack.pop();
            }

            Event::Start(Tag::Emphasis) => {
                let s = ctx.top_style().add_modifier(Modifier::ITALIC);
                ctx.style_stack.push(s);
            }
            Event::End(TagEnd::Emphasis) => {
                ctx.style_stack.pop();
            }

            Event::Start(Tag::Strikethrough) => {
                let s = ctx.top_style().add_modifier(Modifier::CROSSED_OUT);
                ctx.style_stack.push(s);
            }
            Event::End(TagEnd::Strikethrough) => {
                ctx.style_stack.pop();
            }

            Event::Start(Tag::Link { .. }) => {
                ctx.style_stack.push(theme.accent_style());
            }
            Event::End(TagEnd::Link) => {
                ctx.style_stack.pop();
            }

            Event::Code(code) => {
                ctx.segments.push((code.to_string(), theme.success_style()));
            }

            // ── Text content ─────────────────────────────────────
            Event::Text(txt) => {
                if ctx.in_code_block {
                    for raw_line in txt.lines() {
                        ctx.lines.push(highlight_code_line(raw_line, theme));
                    }
                } else {
                    let style = ctx.top_style();
                    ctx.segments.push((txt.to_string(), style));
                }
            }

            Event::SoftBreak if !ctx.in_code_block => {
                let style = ctx.top_style();
                ctx.segments.push((" ".to_string(), style));
            }
            Event::HardBreak => {
                ctx.flush(max_width, theme);
            }

            Event::Rule => {
                if ctx.need_blank_line && !ctx.first_block {
                    ctx.lines.push(Line::from(""));
                }
                let rule_width = max_width.min(60);
                let rule = "\u{2500}".repeat(rule_width);
                ctx.lines
                    .push(Line::from(Span::styled(rule, theme.dim_style())));
                ctx.need_blank_line = true;
                ctx.first_block = false;
            }

            // Skip events we don't render (tables, footnotes, HTML, etc.)
            _ => {}
        }
    }

    // Flush any remaining segments.
    ctx.flush(max_width, theme);

    if ctx.lines.is_empty() {
        ctx.lines.push(Line::from(""));
    }

    ctx.lines
}

// ── Code syntax highlighting ──────────────────────────────────────────

/// Keywords highlighted with bold accent style in code blocks.
const CODE_KEYWORDS: &[&str] = &[
    // Rust
    "fn", "let", "mut", "pub", "use", "struct", "enum", "impl", "trait", "for", "while", "if",
    "else", "match", "return", "self", "Self", "async", "await", "const", "static", "type",
    "where", "mod", "crate", "super", "true", "false", "None", "Some", "Ok", "Err",
    // Python
    "def", "class", "import", "from", "print", // JS/TS
    "var", "function", "export", "default", "require",
];

/// Produce a syntax-highlighted `Line` for a single code-block line.
///
/// Applies basic keyword, string, comment, and number highlighting without
/// any heavy parsing dependency.
fn highlight_code_line(line: &str, theme: &Theme) -> Line<'static> {
    let trimmed = line.trim_start();

    // Full-line comments: `//` or `#` prefix.
    if trimmed.starts_with("//") || trimmed.starts_with('#') {
        return Line::from(Span::styled(line.to_string(), theme.dim_style()));
    }

    let base_style = theme.success_style();
    let keyword_style = Style::default()
        .fg(theme.accent.to_color())
        .add_modifier(Modifier::BOLD);
    let string_style = theme.warning_style();
    let number_style = theme.accent_style();

    let mut spans: Vec<Span<'static>> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // ── Whitespace run ───────────────────────────────────
        if ch.is_whitespace() {
            let start = i;
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(s, base_style));
            continue;
        }

        // ── String literals ──────────────────────────────────
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let start = i;
            i += 1;
            while i < len {
                if chars[i] == '\\' {
                    i += 2; // skip escaped char
                } else if chars[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(s, string_style));
            continue;
        }

        // ── Inline comment (// in the middle of a line) ──────
        if ch == '/' && i + 1 < len && chars[i + 1] == '/' {
            let s: String = chars[i..].iter().collect();
            spans.push(Span::styled(s, theme.dim_style()));
            break;
        }

        // ── Word token (identifier / keyword / number) ───────
        if ch.is_alphanumeric() || ch == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            if CODE_KEYWORDS.contains(&word.as_str()) {
                spans.push(Span::styled(word, keyword_style));
            } else if word.chars().all(|c| c.is_ascii_digit() || c == '_') && !word.is_empty() {
                spans.push(Span::styled(word, number_style));
            } else {
                spans.push(Span::styled(word, base_style));
            }
            continue;
        }

        // ── Punctuation / operators ──────────────────────────
        let start = i;
        while i < len
            && !chars[i].is_whitespace()
            && !chars[i].is_alphanumeric()
            && chars[i] != '_'
            && chars[i] != '"'
            && chars[i] != '\''
            && !(chars[i] == '/' && i + 1 < len && chars[i + 1] == '/')
        {
            i += 1;
        }
        if i == start {
            // Safety: advance at least one character to avoid infinite loop.
            i += 1;
        }
        let s: String = chars[start..i].iter().collect();
        spans.push(Span::styled(s, base_style));
    }

    if spans.is_empty() {
        Line::from(Span::styled(String::new(), base_style))
    } else {
        Line::from(spans)
    }
}

/// Internal state for the markdown event walker.
struct MdContext {
    style_stack: Vec<Style>,
    segments: Vec<(String, Style)>,
    lines: Vec<Line<'static>>,
    list_stack: Vec<ListKind>,
    in_code_block: bool,
    in_blockquote: bool,
    need_blank_line: bool,
    first_block: bool,
}

impl MdContext {
    fn new(theme: &Theme) -> Self {
        Self {
            style_stack: vec![Style::default().fg(theme.fg.to_color())],
            segments: Vec::new(),
            lines: Vec::new(),
            list_stack: Vec::new(),
            in_code_block: false,
            in_blockquote: false,
            need_blank_line: false,
            first_block: true,
        }
    }

    /// Current top-of-stack style.
    fn top_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    /// Flush accumulated segments into word-wrapped lines.
    fn flush(&mut self, max_width: usize, theme: &Theme) {
        if self.segments.is_empty() {
            return;
        }
        let indent = list_indent(&self.list_stack);
        let wrapped = wrap_styled_segments(
            std::mem::take(&mut self.segments),
            max_width.saturating_sub(indent),
            self.in_blockquote,
            theme,
        );
        for mut line in wrapped {
            if indent > 0 {
                let pad = " ".repeat(indent);
                line.spans.insert(0, Span::raw(pad));
            }
            self.lines.push(line);
        }
    }
}

/// Compute continuation-line indent based on current list nesting.
fn list_indent(list_stack: &[ListKind]) -> usize {
    if list_stack.is_empty() {
        0
    } else {
        (list_stack.len().saturating_sub(1)) * 4
    }
}

/// Word-wrap a sequence of styled text segments into `Line`s, respecting
/// `max_width` using `unicode_width` for correct CJK/emoji sizing.
///
/// If `in_blockquote` is true, each line is prefixed with a dim `\u{2502} `.
fn wrap_styled_segments(
    segments: Vec<(String, Style)>,
    max_width: usize,
    in_blockquote: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![];
    }

    let bq_prefix_width: usize = if in_blockquote { 2 } else { 0 };
    let effective_width = max_width.saturating_sub(bq_prefix_width);
    if effective_width == 0 {
        return vec![];
    }

    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width: usize = 0;

    for (text, style) in segments {
        let words: Vec<&str> = text.split(' ').collect();
        for (i, word) in words.iter().enumerate() {
            if word.is_empty() && i > 0 {
                // Preserve a single space between segments.
                if current_width < effective_width && current_width > 0 {
                    current_spans.push(Span::styled(" ".to_string(), style));
                    current_width += 1;
                }
                continue;
            }
            let w = UnicodeWidthStr::width(*word);
            if w == 0 && word.is_empty() {
                continue;
            }

            // Need a space separator?
            let need_space = current_width > 0 && i > 0;
            let space_w: usize = if need_space { 1 } else { 0 };

            if current_width + space_w + w <= effective_width {
                if need_space {
                    current_spans.push(Span::styled(" ".to_string(), style));
                    current_width += 1;
                }
                current_spans.push(Span::styled(word.to_string(), style));
                current_width += w;
            } else if current_width == 0 {
                // Word wider than max_width — emit as-is on its own line.
                current_spans.push(Span::styled(word.to_string(), style));
                lines.push(std::mem::take(&mut current_spans));
                current_width = 0;
            } else {
                // Wrap: finish current line, start new one.
                lines.push(std::mem::take(&mut current_spans));
                current_spans.push(Span::styled(word.to_string(), style));
                current_width = w;
            }
        }
    }

    if !current_spans.is_empty() {
        lines.push(current_spans);
    }

    if lines.is_empty() {
        lines.push(vec![Span::raw(String::new())]);
    }

    // Apply blockquote prefix if needed.
    lines
        .into_iter()
        .map(|spans| {
            if in_blockquote {
                let mut prefixed = vec![Span::styled("\u{2502} ".to_string(), theme.dim_style())];
                prefixed.extend(spans);
                Line::from(prefixed)
            } else {
                Line::from(spans)
            }
        })
        .collect()
}

/// Truncate a string to a maximum character count, appending "..." if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

/// Collapse a multi-line preview into a single line for inline display.
///
/// Replaces newlines with spaces and collapses consecutive whitespace,
/// then truncates to `max` characters.
pub fn collapse_preview(s: &str, max: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&collapsed, max)
}

/// Format a duration in seconds to a human-readable string (e.g., "2m", "1h 5m").
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h")
        }
    }
}

/// Format a tool duration in milliseconds (e.g., "37ms", "1.3s", "2m 5s").
pub fn format_tool_duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        let m = secs / 60;
        let s = secs % 60;
        if s > 0 {
            format!("{m}m {s}s")
        } else {
            format!("{m}m")
        }
    }
}

/// Format a token count with K/M suffix.
pub fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    // ── wrap_text tests ─────────────────────────────────────────

    #[test]
    fn wrap_text_no_wrapping_needed() {
        let lines = wrap_text("short line", 80, Style::default());
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn wrap_text_wraps_long_line() {
        let text = "the quick brown fox jumps over the lazy dog";
        let lines = wrap_text(text, 20, Style::default());
        assert!(lines.len() > 1);
    }

    #[test]
    fn wrap_text_preserves_consecutive_spaces() {
        let lines = wrap_text("keep  double   spaces", 80, Style::default());
        assert_eq!(line_text(&lines[0]), "keep  double   spaces");
    }

    #[test]
    fn wrap_text_empty() {
        let lines = wrap_text("", 80, Style::default());
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn wrap_text_zero_width() {
        let lines = wrap_text("hello", 0, Style::default());
        assert!(lines.is_empty());
    }

    // ── truncate / format helpers ───────────────────────────────

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("hello world this is a test", 10);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 10);
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(120), "2m");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3660), "1h 1m");
    }

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(500), "500");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(2100), "2.1K");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    // ── render_markdown tests ───────────────────────────────────

    /// Collect all text content from a slice of Lines into a single string.
    fn lines_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Check whether any span in any line has the given modifier.
    fn has_modifier(lines: &[Line<'_>], modifier: Modifier) -> bool {
        lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(modifier))
        })
    }

    /// Check whether any span in any line has the given foreground color.
    fn has_fg(lines: &[Line<'_>], color: ratatui::style::Color) -> bool {
        lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.style.fg == Some(color)))
    }

    #[test]
    fn md_plain_text() {
        let theme = Theme::dark();
        let lines = render_markdown("Hello world", 80, &theme);
        assert!(lines_text(&lines).contains("Hello world"));
    }

    #[test]
    fn md_empty_input() {
        let theme = Theme::dark();
        let lines = render_markdown("", 80, &theme);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn md_zero_width() {
        let theme = Theme::dark();
        let lines = render_markdown("hello", 0, &theme);
        assert!(lines.is_empty());
    }

    #[test]
    fn md_bold() {
        let theme = Theme::dark();
        let lines = render_markdown("some **bold** text", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("bold"));
        assert!(has_modifier(&lines, Modifier::BOLD));
    }

    #[test]
    fn md_italic() {
        let theme = Theme::dark();
        let lines = render_markdown("some *italic* text", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("italic"));
        assert!(has_modifier(&lines, Modifier::ITALIC));
    }

    #[test]
    fn md_inline_code() {
        let theme = Theme::dark();
        let lines = render_markdown("run `cargo test` now", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("cargo test"));
        assert!(has_fg(&lines, theme.success.to_color()));
    }

    #[test]
    fn md_heading_h1() {
        let theme = Theme::dark();
        let lines = render_markdown("# Title", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("Title"));
        assert!(has_modifier(&lines, Modifier::BOLD));
        assert!(has_fg(&lines, theme.accent.to_color()));
    }

    #[test]
    fn md_heading_h2() {
        let theme = Theme::dark();
        let lines = render_markdown("## Subtitle", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("Subtitle"));
        assert!(has_modifier(&lines, Modifier::BOLD));
        assert!(has_fg(&lines, theme.accent.to_color()));
    }

    #[test]
    fn md_heading_h3() {
        let theme = Theme::dark();
        let lines = render_markdown("### Section", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("Section"));
        assert!(has_modifier(&lines, Modifier::BOLD));
    }

    #[test]
    fn md_unordered_list() {
        let theme = Theme::dark();
        let lines = render_markdown("- alpha\n- beta\n- gamma", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("\u{2022} alpha"));
        assert!(text.contains("\u{2022} beta"));
        assert!(text.contains("\u{2022} gamma"));
    }

    #[test]
    fn md_ordered_list() {
        let theme = Theme::dark();
        let lines = render_markdown("1. first\n2. second\n3. third", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("1. first"));
        assert!(text.contains("2. second"));
        assert!(text.contains("3. third"));
    }

    #[test]
    fn md_code_block() {
        let theme = Theme::dark();
        let lines = render_markdown("```rust\nfn main() {}\n```", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("fn main() {}"));
        assert!(has_fg(&lines, theme.success.to_color()));
    }

    #[test]
    fn md_blockquote() {
        let theme = Theme::dark();
        let lines = render_markdown("> quoted text", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("\u{2502} "));
        assert!(text.contains("quoted text"));
    }

    #[test]
    fn md_horizontal_rule() {
        let theme = Theme::dark();
        let lines = render_markdown("above\n\n---\n\nbelow", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("\u{2500}"));
        assert!(text.contains("above"));
        assert!(text.contains("below"));
    }

    #[test]
    fn md_link() {
        let theme = Theme::dark();
        let lines = render_markdown("[click here](https://example.com)", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("click here"));
        // URL itself should not appear in text.
        assert!(!text.contains("https://example.com"));
        assert!(has_fg(&lines, theme.accent.to_color()));
    }

    #[test]
    fn md_nested_bold_italic() {
        let theme = Theme::dark();
        let lines = render_markdown("***bold and italic***", 80, &theme);
        // Should have both modifiers.
        assert!(has_modifier(&lines, Modifier::BOLD));
        assert!(has_modifier(&lines, Modifier::ITALIC));
    }

    #[test]
    fn md_word_wrap() {
        let theme = Theme::dark();
        let text = "The quick brown fox jumps over the lazy dog near the river bank";
        let lines = render_markdown(text, 20, &theme);
        // Should produce multiple lines.
        assert!(lines.len() > 1);
        // All words should still be present.
        let joined = lines_text(&lines);
        assert!(joined.contains("quick"));
        assert!(joined.contains("dog"));
    }

    #[test]
    fn md_realistic_response() {
        let theme = Theme::dark();
        let md = "\
## Summary

Here is a **bold** claim with `inline code`.

- First item
- Second item with *emphasis*

```python
def hello():
    print(\"world\")
```

> A wise quote

---

That's all!";
        let lines = render_markdown(md, 60, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("Summary"));
        assert!(text.contains("bold"));
        assert!(text.contains("inline code"));
        assert!(text.contains("\u{2022} First item"));
        assert!(text.contains("def hello():"));
        assert!(text.contains("\u{2502} "));
        assert!(text.contains("\u{2500}"));
        assert!(text.contains("That's all!"));
    }

    #[test]
    fn md_paragraph_separation() {
        let theme = Theme::dark();
        let lines = render_markdown("First paragraph.\n\nSecond paragraph.", 80, &theme);
        // There should be a blank line between paragraphs.
        let blank_count = lines
            .iter()
            .filter(|l| lines_text(&[(*l).clone()]).is_empty())
            .count();
        assert!(blank_count >= 1, "expected blank line between paragraphs");
    }

    #[test]
    fn md_strikethrough() {
        let theme = Theme::dark();
        let lines = render_markdown("~~deleted~~", 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("deleted"));
        assert!(has_modifier(&lines, Modifier::CROSSED_OUT));
    }

    // ── highlight_code_line tests ──────────────────────────────

    #[test]
    fn highlight_comment_line() {
        let theme = Theme::dark();
        let line = highlight_code_line("    // this is a comment", &theme);
        // Entire line should be dim (one span).
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style, theme.dim_style());
    }

    #[test]
    fn highlight_hash_comment() {
        let theme = Theme::dark();
        let line = highlight_code_line("# comment", &theme);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style, theme.dim_style());
    }

    #[test]
    fn highlight_keyword() {
        let theme = Theme::dark();
        let line = highlight_code_line("fn main() {}", &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("fn"));
        // The "fn" span should have bold + accent color.
        let fn_span = line.spans.iter().find(|s| s.content.as_ref() == "fn");
        assert!(fn_span.is_some());
        let style = fn_span.unwrap().style;
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(style.fg, Some(theme.accent.to_color()));
    }

    #[test]
    fn highlight_string_literal() {
        let theme = Theme::dark();
        let line = highlight_code_line("let x = \"hello world\";", &theme);
        let str_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("hello"));
        assert!(str_span.is_some());
        assert_eq!(str_span.unwrap().style, theme.warning_style());
    }

    #[test]
    fn highlight_number() {
        let theme = Theme::dark();
        let line = highlight_code_line("let x = 42;", &theme);
        let num_span = line.spans.iter().find(|s| s.content.as_ref() == "42");
        assert!(num_span.is_some());
        assert_eq!(num_span.unwrap().style, theme.accent_style());
    }

    #[test]
    fn highlight_preserves_indentation() {
        let theme = Theme::dark();
        let line = highlight_code_line("    let x = 1;", &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("    "));
    }

    #[test]
    fn highlight_inline_comment() {
        let theme = Theme::dark();
        let line = highlight_code_line("let x = 1; // count", &theme);
        let comment_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("// count"));
        assert!(comment_span.is_some());
        assert_eq!(comment_span.unwrap().style, theme.dim_style());
    }

    #[test]
    fn highlight_empty_line() {
        let theme = Theme::dark();
        let line = highlight_code_line("", &theme);
        // Should produce a valid (possibly empty) line without panicking.
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.is_empty());
    }

    #[test]
    fn highlight_python_keywords() {
        let theme = Theme::dark();
        let line = highlight_code_line("def hello():", &theme);
        let def_span = line.spans.iter().find(|s| s.content.as_ref() == "def");
        assert!(def_span.is_some());
        assert!(
            def_span
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn highlight_code_block_integration() {
        let theme = Theme::dark();
        let md = "```rust\nfn main() {\n    let x = 42;\n}\n```";
        let lines = render_markdown(md, 80, &theme);
        // "fn" keyword should be bold accent, not plain green.
        let has_bold_accent = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.content.as_ref() == "fn"
                    && s.style.add_modifier.contains(Modifier::BOLD)
                    && s.style.fg == Some(theme.accent.to_color())
            })
        });
        assert!(
            has_bold_accent,
            "expected 'fn' to be highlighted as keyword"
        );
    }
}
