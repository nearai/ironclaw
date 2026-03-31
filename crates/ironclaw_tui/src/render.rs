//! Rendering utilities for converting text to styled Ratatui spans.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Convert a plain text string into wrapped `Line`s that fit within `max_width`.
pub fn wrap_text<'a>(text: &'a str, max_width: usize, style: Style) -> Vec<Line<'a>> {
    if max_width == 0 {
        return vec![];
    }

    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        // Simple word-wrap
        let words: Vec<&str> = raw_line.split_whitespace().collect();
        if words.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        let mut current = String::new();
        for word in words {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(Line::from(Span::styled(current, style)));
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(Line::from(Span::styled(current, style)));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
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

            Event::Start(Tag::Paragraph) => {
                if ctx.need_blank_line && !ctx.first_block {
                    ctx.lines.push(Line::from(""));
                    ctx.need_blank_line = false;
                }
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

            Event::Start(Tag::CodeBlock(_)) => {
                if ctx.need_blank_line && !ctx.first_block {
                    ctx.lines.push(Line::from(""));
                    ctx.need_blank_line = false;
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
                    let code_style = theme.success_style();
                    for raw_line in txt.lines() {
                        ctx.lines
                            .push(Line::from(Span::styled(raw_line.to_string(), code_style)));
                    }
                } else {
                    let style = ctx.top_style();
                    ctx.segments.push((txt.to_string(), style));
                }
            }

            Event::SoftBreak => {
                if !ctx.in_code_block {
                    let style = ctx.top_style();
                    ctx.segments.push((" ".to_string(), style));
                }
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
}
