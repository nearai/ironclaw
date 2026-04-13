//! Rich inline code block rendering for tool output.
//!
//! Renders Read/Write/Edit/Bash tool output as Claude Code-style code blocks
//! with syntax highlighting, line numbers, colored diffs, and collapse support.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::render::{format_tool_duration, highlight_code_line_enhanced, infer_language_from_path};
use crate::theme::Theme;

use super::ToolActivity;

/// Maximum lines shown before collapsing.
const COLLAPSED_LINES: usize = 3;
/// Width reserved for line numbers (right-aligned).
const LINE_NUM_WIDTH: usize = 4;

/// Render a tool's output as a rich code block for the conversation.
///
/// Returns styled `Line`s ready to embed in the conversation widget.
pub fn render_tool_block(
    tool: &ToolActivity,
    usable_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let display = format_display_name(&tool.name);
    let is_bash = matches!(display, "Bash");

    // Header line: ● Name(detail)  duration
    lines.push(render_tool_header(tool, usable_width, theme));

    // Status line and content for completed tools
    if let Some(ref preview) = tool.result_preview
        && !preview.is_empty()
    {
        lines.push(render_status_line(tool, theme));
        lines.push(Line::from("")); // blank line before content

        let content_lines: Vec<&str> = preview.lines().collect();
        if content_lines.is_empty() {
            return lines;
        }

        if is_bash {
            render_bash_content(tool, &content_lines, usable_width, theme, &mut lines);
        } else if looks_like_diff(preview) {
            render_diff_content(
                &content_lines,
                usable_width,
                tool.expanded,
                theme,
                &mut lines,
            );
        } else {
            let language = tool.detail.as_deref().and_then(infer_language_from_path);
            render_code_content(
                &content_lines,
                usable_width,
                tool.expanded,
                language.as_deref(),
                theme,
                &mut lines,
            );
        }

        // Collapse indicator (not for bash — handled inside render_bash_content)
        if !is_bash && !tool.expanded && content_lines.len() > COLLAPSED_LINES {
            let remaining = content_lines.len() - COLLAPSED_LINES;
            lines.push(Line::from(Span::styled(
                format!("    \u{2026} +{remaining} lines (ctrl+e to expand)"),
                Style::default()
                    .fg(theme.dim.to_color())
                    .add_modifier(Modifier::ITALIC),
            )));
        }

        // Collapse hint when expanded (not for bash — handled inside render_bash_content)
        if !is_bash && tool.expanded && content_lines.len() > COLLAPSED_LINES {
            lines.push(Line::from(Span::styled(
                "    \u{25BE} ctrl+e to collapse".to_string(),
                Style::default()
                    .fg(theme.dim.to_color())
                    .add_modifier(Modifier::ITALIC),
            )));
        }
    }

    lines
}

/// Map internal tool names to human-readable display names.
///
/// Order matters: more specific matches (e.g. "memory") must come before
/// generic ones (e.g. "write") to avoid `memory_write` matching as "Write".
pub fn format_display_name(tool_name: &str) -> &'static str {
    let lower = tool_name.to_lowercase();
    // Specific compound names first
    if lower.contains("memory") {
        "Memory"
    } else if lower.contains("mcp") {
        "MCP"
    } else if lower.contains("glob") || lower.contains("find") || lower.contains("list_dir") {
        "Glob"
    } else if lower.contains("grep") || lower.contains("ripgrep") {
        "Search"
    // Then the core file/shell operations
    } else if lower.contains("read") || lower == "cat" {
        "Read"
    } else if lower.contains("write") {
        "Write"
    } else if lower.contains("edit") || lower.contains("patch") {
        "Edit"
    } else if lower.contains("shell")
        || lower.contains("bash")
        || lower.contains("exec")
        || lower.contains("command")
    {
        "Bash"
    } else if lower.contains("http") || lower.contains("web") || lower.contains("fetch") {
        "Fetch"
    } else if lower.contains("search") {
        "Search"
    } else {
        "Tool"
    }
}

/// Render the header line: ` ● Name(detail)  duration`
fn render_tool_header(tool: &ToolActivity, usable_width: usize, theme: &Theme) -> Line<'static> {
    let display = format_display_name(&tool.name);
    let is_read = display == "Read";

    let dot_style = if is_read {
        theme.tool_read_dot_style()
    } else {
        theme.tool_action_dot_style()
    };

    let detail_text = tool
        .detail
        .as_deref()
        .map(|d| format!("({d})"))
        .unwrap_or_default();

    let duration_text = tool
        .duration_ms
        .map(format_tool_duration)
        .unwrap_or_default();

    // " ● Name(detail)" + right-aligned duration
    let left = format!(" \u{25CF} {display}{detail_text}");
    let left_len = left.len();
    let dur_len = duration_text.len();
    let gap = usable_width.saturating_sub(left_len + dur_len).max(1);

    Line::from(vec![
        Span::styled(" \u{25CF} ", dot_style),
        Span::styled(
            display.to_string(),
            Style::default()
                .fg(theme.fg.to_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(detail_text, theme.dim_style()),
        Span::raw(" ".repeat(gap)),
        Span::styled(duration_text, theme.dim_style()),
    ])
}

/// Render the status line: `  ↳ Status message`
fn render_status_line(tool: &ToolActivity, theme: &Theme) -> Line<'static> {
    let display = format_display_name(&tool.name);
    let status_msg = match display {
        "Read" => {
            let line_count = tool
                .result_preview
                .as_ref()
                .map(|p| p.lines().count())
                .unwrap_or(0);
            format!("Read {line_count} lines")
        }
        "Write" => {
            let line_count = tool
                .result_preview
                .as_ref()
                .map(|p| p.lines().count())
                .unwrap_or(0);
            let path = tool.detail.as_deref().unwrap_or("file");
            format!("Wrote {line_count} lines to {path}")
        }
        "Edit" => "Applied edit".to_string(),
        "Bash" => {
            if let Some(ms) = tool.duration_ms {
                format!("Completed in {}", format_tool_duration(ms))
            } else {
                "Completed".to_string()
            }
        }
        _ => "Done".to_string(),
    };

    Line::from(vec![
        Span::styled("   \u{21B3} ".to_string(), theme.dim_style()),
        Span::styled(status_msg, theme.dim_style()),
    ])
}

/// Render syntax-highlighted code with line numbers.
fn render_code_content(
    content_lines: &[&str],
    _usable_width: usize,
    expanded: bool,
    language: Option<&str>,
    theme: &Theme,
    output: &mut Vec<Line<'static>>,
) {
    let max_lines = if expanded {
        content_lines.len()
    } else {
        content_lines.len().min(COLLAPSED_LINES)
    };

    for (idx, line) in content_lines.iter().take(max_lines).enumerate() {
        let line_num = idx + 1;
        let num_str = format!("{:>width$}  ", line_num, width = LINE_NUM_WIDTH);

        let highlighted = highlight_code_line_enhanced(line, language, theme);

        let mut spans = vec![
            Span::raw("    ".to_string()), // indent
            Span::styled(num_str, theme.line_number_style()),
        ];
        spans.extend(highlighted.spans);
        output.push(Line::from(spans));
    }
}

/// Render diff content with green additions and red deletions.
fn render_diff_content(
    content_lines: &[&str],
    _usable_width: usize,
    expanded: bool,
    theme: &Theme,
    output: &mut Vec<Line<'static>>,
) {
    let max_lines = if expanded {
        content_lines.len()
    } else {
        content_lines.len().min(COLLAPSED_LINES)
    };

    let mut line_num = 0u32;

    for line in content_lines.iter().take(max_lines) {
        if line.starts_with("@@") {
            // Hunk header
            output.push(Line::from(vec![
                Span::raw("   ".to_string()),
                Span::styled((*line).to_string(), theme.diff_hunk_style()),
            ]));
            // Try to parse starting line number from @@ -N,... +M,...
            if let Some(plus_part) = line.split('+').nth(1)
                && let Some(num_str) = plus_part
                    .split(',')
                    .next()
                    .or_else(|| plus_part.split(' ').next())
            {
                line_num = num_str.parse::<u32>().unwrap_or(0);
                line_num = line_num.saturating_sub(1); // will be incremented below
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix('+') {
            line_num += 1;
            let num_str = format!("{:>width$} ", line_num, width = LINE_NUM_WIDTH);
            output.push(Line::from(vec![
                Span::raw("   ".to_string()),
                Span::styled(num_str, theme.diff_add_marker_style()),
                Span::styled("+ ", theme.diff_add_marker_style()),
                Span::styled(rest.to_string(), theme.diff_add_style()),
            ]));
        } else if let Some(rest) = line.strip_prefix('-') {
            let num_str = format!("{:>width$} ", " ", width = LINE_NUM_WIDTH);
            output.push(Line::from(vec![
                Span::raw("   ".to_string()),
                Span::styled(num_str, theme.diff_del_marker_style()),
                Span::styled("- ", theme.diff_del_marker_style()),
                Span::styled(rest.to_string(), theme.diff_del_style()),
            ]));
        } else {
            // Context line
            line_num += 1;
            let num_str = format!("{:>width$}  ", line_num, width = LINE_NUM_WIDTH);
            let text = line.strip_prefix(' ').unwrap_or(line);
            output.push(Line::from(vec![
                Span::raw("   ".to_string()),
                Span::styled(num_str, theme.line_number_style()),
                Span::styled(text.to_string(), theme.dim_style()),
            ]));
        }
    }
}

/// Render bash command output with `$ command` header and dim output.
fn render_bash_content(
    tool: &ToolActivity,
    content_lines: &[&str],
    _usable_width: usize,
    theme: &Theme,
    output: &mut Vec<Line<'static>>,
) {
    // Command header
    let command = tool.detail.as_deref().unwrap_or(&tool.name);
    output.push(Line::from(vec![
        Span::raw("   ".to_string()),
        Span::styled(
            "$ ".to_string(),
            Style::default()
                .fg(theme.fg.to_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            command.to_string(),
            Style::default()
                .fg(theme.fg.to_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Output lines (no line numbers, dim)
    let max_lines = if tool.expanded {
        content_lines.len()
    } else {
        content_lines.len().min(COLLAPSED_LINES)
    };

    for line in content_lines.iter().take(max_lines) {
        output.push(Line::from(vec![
            Span::raw("   ".to_string()),
            Span::styled((*line).to_string(), theme.dim_style()),
        ]));
    }

    // Collapse indicator for bash
    if !tool.expanded && content_lines.len() > COLLAPSED_LINES {
        let remaining = content_lines.len() - COLLAPSED_LINES;
        output.push(Line::from(Span::styled(
            format!("    \u{2026} +{remaining} lines (ctrl+e to expand)"),
            Style::default()
                .fg(theme.dim.to_color())
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Collapse hint when expanded for bash
    if tool.expanded && content_lines.len() > COLLAPSED_LINES {
        output.push(Line::from(Span::styled(
            "    \u{25BE} ctrl+e to collapse".to_string(),
            Style::default()
                .fg(theme.dim.to_color())
                .add_modifier(Modifier::ITALIC),
        )));
    }
}

/// Heuristic to detect if content looks like a unified diff.
fn looks_like_diff(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().take(20).collect();
    if lines.is_empty() {
        return false;
    }
    let diff_markers = lines
        .iter()
        .filter(|l| l.starts_with('+') || l.starts_with('-') || l.starts_with("@@"))
        .count();
    diff_markers >= 2 && (diff_markers as f32 / lines.len() as f32) > 0.2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::ToolStatus;

    fn make_tool(name: &str, detail: Option<&str>, preview: Option<&str>) -> ToolActivity {
        ToolActivity {
            call_id: None,
            name: name.to_string(),
            started_at: chrono::Utc::now(),
            duration_ms: Some(150),
            status: ToolStatus::Success,
            detail: detail.map(|s| s.to_string()),
            result_preview: preview.map(|s| s.to_string()),
            expanded: false,
        }
    }

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

    #[test]
    fn display_name_mapping() {
        assert_eq!(format_display_name("read_file"), "Read");
        assert_eq!(format_display_name("write_file"), "Write");
        assert_eq!(format_display_name("apply_patch"), "Edit");
        assert_eq!(format_display_name("file_edit"), "Edit");
        assert_eq!(format_display_name("shell"), "Bash");
        assert_eq!(format_display_name("execute_command"), "Bash");
        assert_eq!(format_display_name("bash"), "Bash");
        assert_eq!(format_display_name("glob_files"), "Glob");
        assert_eq!(format_display_name("grep_search"), "Search");
        assert_eq!(format_display_name("memory_write"), "Memory");
        assert_eq!(format_display_name("web_fetch"), "Fetch");
        assert_eq!(format_display_name("mcp_call"), "MCP");
        assert_eq!(format_display_name("unknown_tool"), "Tool");
    }

    #[test]
    fn diff_detection_positive() {
        let diff = "@@ -1,3 +1,5 @@\n line1\n+added\n-removed\n line2";
        assert!(looks_like_diff(diff));
    }

    #[test]
    fn diff_detection_negative() {
        let code = "fn main() {\n    println!(\"hello\");\n}\n";
        assert!(!looks_like_diff(code));
    }

    #[test]
    fn render_read_tool_block() {
        let theme = Theme::dark();
        let tool = make_tool(
            "read_file",
            Some("src/main.rs"),
            Some("use std::io;\n\nfn main() {\n    println!(\"hello\");\n}"),
        );
        let lines = render_tool_block(&tool, 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("\u{25CF}")); // dot
        assert!(text.contains("Read"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("fn"));
        assert!(text.contains("main"));
    }

    #[test]
    fn render_bash_tool_block() {
        let theme = Theme::dark();
        let tool = make_tool(
            "shell",
            Some("cargo test"),
            Some("running 5 tests\ntest result: ok"),
        );
        let lines = render_tool_block(&tool, 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("Bash"));
        assert!(text.contains("$ cargo test"));
    }

    #[test]
    fn render_edit_with_diff() {
        let theme = Theme::dark();
        let diff = "@@ -1,3 +1,4 @@\n line1\n+added line\n line2";
        let tool = make_tool("file_edit", Some("src/lib.rs"), Some(diff));
        let lines = render_tool_block(&tool, 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("Edit"));
        assert!(text.contains("@@"));
        assert!(text.contains("+ added line"));
    }

    #[test]
    fn collapse_long_content() {
        let theme = Theme::dark();
        let content = (1..=30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tool = make_tool("read_file", Some("big.rs"), Some(&content));
        let lines = render_tool_block(&tool, 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("+27 lines"));
        assert!(text.contains("ctrl+e to expand"));
    }

    #[test]
    fn expanded_shows_all_lines() {
        let theme = Theme::dark();
        let content = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut tool = make_tool("read_file", Some("big.rs"), Some(&content));
        tool.expanded = true;
        let lines = render_tool_block(&tool, 80, &theme);
        let text = lines_text(&lines);
        assert!(text.contains("line 20"));
        assert!(!text.contains("ctrl+e to expand"));
        assert!(text.contains("ctrl+e to collapse"));
    }

    #[test]
    fn no_preview_renders_header_only() {
        let theme = Theme::dark();
        let tool = make_tool("read_file", Some("src/main.rs"), None);
        let lines = render_tool_block(&tool, 80, &theme);
        assert!(lines.len() <= 2); // header only, maybe an empty line
    }
}
