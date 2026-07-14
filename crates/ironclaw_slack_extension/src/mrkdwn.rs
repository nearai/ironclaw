//! Slack mrkdwn rendering helpers.

/// Slack truncates very large `chat.postMessage` payloads. Keep chunks below
/// the documented hard ceiling and leave room for the part header we add when
/// a final reply spans multiple messages.
const SLACK_TEXT_SOFT_LIMIT_CHARS: usize = 35_000;
const SLACK_TEXT_CHUNK_BODY_CHARS: usize = 34_900;

pub(crate) fn render_slack_mrkdwn(markdown: &str) -> String {
    let mut rendered = String::with_capacity(markdown.len());
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        if let Some((headers, row_start)) = markdown_table_header_at(&lines, index) {
            let mut row_index = row_start;
            let mut rows = Vec::new();
            while row_index < lines.len() {
                let Some(cells) = split_pipe_cells(lines[row_index]) else {
                    break;
                };
                if is_markdown_table_separator_cells(&cells) {
                    row_index += 1;
                    continue;
                }
                rows.push(normalize_table_row_cells(headers.len(), cells));
                row_index += 1;
            }
            if !rows.is_empty() {
                for (row_offset, row) in rows.iter().enumerate() {
                    rendered.push_str(&render_slack_table_record(&headers, row));
                    if row_offset + 1 < rows.len() || row_index < lines.len() {
                        rendered.push('\n');
                    }
                }
                index = row_index;
                continue;
            }
        }
        if is_markdown_table_separator(lines[index]) {
            index += 1;
            continue;
        }
        let line = lines[index];
        let converted = if is_markdown_table_row(line) {
            render_table_row(line)
        } else {
            render_slack_mrkdwn_line(line)
        };
        rendered.push_str(&converted);
        if index + 1 < lines.len() {
            rendered.push('\n');
        }
        index += 1;
    }
    rendered
}

pub(crate) fn slack_text_chunks(text: &str) -> Vec<String> {
    if text.chars().count() <= SLACK_TEXT_SOFT_LIMIT_CHARS {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;

    for segment in text.split_inclusive('\n') {
        push_slack_text_segment(
            segment,
            &mut current,
            &mut current_chars,
            &mut chunks,
            SLACK_TEXT_CHUNK_BODY_CHARS,
        );
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }

    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| format!("Part {}/{}\n{}", index + 1, total, chunk))
        .collect()
}

fn push_slack_text_segment(
    segment: &str,
    current: &mut String,
    current_chars: &mut usize,
    chunks: &mut Vec<String>,
    limit: usize,
) {
    let segment_chars = segment.chars().count();
    if segment_chars > limit {
        flush_slack_text_chunk(current, current_chars, chunks);
        for ch in segment.chars() {
            current.push(ch);
            *current_chars += 1;
            if *current_chars >= limit {
                flush_slack_text_chunk(current, current_chars, chunks);
            }
        }
        return;
    }

    if *current_chars > 0 && *current_chars + segment_chars > limit {
        flush_slack_text_chunk(current, current_chars, chunks);
    }
    current.push_str(segment);
    *current_chars += segment_chars;
}

fn flush_slack_text_chunk(
    current: &mut String,
    current_chars: &mut usize,
    chunks: &mut Vec<String>,
) {
    if current.is_empty() {
        return;
    }
    chunks.push(std::mem::take(current));
    *current_chars = 0;
}

fn markdown_table_header_at(lines: &[&str], index: usize) -> Option<(Vec<String>, usize)> {
    let cells = split_pipe_cells(lines[index])?;
    let next_is_separator = lines
        .get(index + 1)
        .and_then(|line| split_pipe_cells(line))
        .is_some_and(|cells| is_markdown_table_separator_cells(&cells));
    if !(is_issue_table_header(&cells) || is_markdown_table_row(lines[index]) && next_is_separator)
    {
        return None;
    }
    let row_start = if next_is_separator {
        index + 2
    } else {
        index + 1
    };
    Some((cells, row_start))
}

fn split_pipe_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let cells = trimmed
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect::<Vec<_>>();
    (cells.len() >= 2).then_some(cells)
}

fn normalize_table_row_cells(header_len: usize, mut cells: Vec<String>) -> Vec<String> {
    if cells.len() > header_len && header_len > 0 {
        let overflow = cells.split_off(header_len - 1);
        cells.push(overflow.join(" | "));
    }
    while cells.len() < header_len {
        cells.push(String::new());
    }
    cells
}

fn is_issue_table_header(cells: &[String]) -> bool {
    let normalized = cells
        .iter()
        .map(|cell| normalized_table_header(cell))
        .collect::<Vec<_>>();
    normalized
        .first()
        .is_some_and(|cell| cell == "#" || cell == "issue")
        && normalized.iter().any(|cell| cell == "title")
}

fn normalized_table_header(cell: &str) -> String {
    cell.trim()
        .trim_matches('*')
        .trim_matches('`')
        .trim()
        .to_ascii_lowercase()
}

fn is_markdown_table_separator_cells(cells: &[String]) -> bool {
    cells.iter().all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
    })
}

fn render_slack_table_record(headers: &[String], row: &[String]) -> String {
    if is_issue_table_header(headers) {
        return render_issue_table_record(headers, row);
    }

    let mut output = String::new();
    for (index, (header, cell)) in headers.iter().zip(row.iter()).enumerate() {
        let cell = render_slack_mrkdwn_line(cell).trim().to_string();
        if cell.is_empty() || cell == "-" {
            continue;
        }
        let header = render_slack_mrkdwn_line(header).trim().to_string();
        if output.is_empty() {
            output.push_str("• *");
            output.push_str(&header);
            output.push_str(":* ");
            output.push_str(&cell);
        } else {
            output.push('\n');
            output.push_str("  *");
            output.push_str(&header);
            output.push_str(":* ");
            output.push_str(&cell);
        }
        if index == 0 && headers.len() == 1 {
            break;
        }
    }
    if output.is_empty() {
        return "•".to_string();
    }
    output
}

fn render_issue_table_record(headers: &[String], row: &[String]) -> String {
    let issue = row.first().map_or("", String::as_str);
    let title = cell_for_header(headers, row, "title").unwrap_or("");
    let mut output = String::from("• ");
    output.push_str(&render_issue_reference(issue));
    let title = render_slack_mrkdwn_line(title).trim().to_string();
    if !title.is_empty() && title != "-" {
        output.push(' ');
        output.push_str(&title);
    }

    for (header, cell) in headers.iter().zip(row.iter()).skip(1) {
        let normalized = normalized_table_header(header);
        if normalized == "title" {
            continue;
        }
        let cell = render_slack_mrkdwn_line(cell).trim().to_string();
        if cell.is_empty() || cell == "-" {
            continue;
        }
        output.push('\n');
        output.push_str("  *");
        output.push_str(issue_detail_label(&normalized, header));
        output.push_str(":* ");
        output.push_str(&cell);
    }
    output
}

fn cell_for_header<'a>(headers: &[String], row: &'a [String], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .position(|header| normalized_table_header(header) == name)
        .and_then(|index| row.get(index))
        .map(String::as_str)
}

fn issue_detail_label<'a>(normalized: &str, original: &'a str) -> &'a str {
    match normalized {
        "status / summary" | "status/summary" | "summary" => "Summary",
        "assignees" => "Assignees",
        "labels" => "Labels",
        "updated" => "Updated",
        _ => original.trim(),
    }
}

fn render_issue_reference(issue: &str) -> String {
    let rendered = render_slack_mrkdwn_line(issue).trim().to_string();
    if let Some((url, label)) = parse_slack_link(&rendered)
        && label.chars().all(|ch| ch.is_ascii_digit())
    {
        return format!("<{url}|#{label}>");
    }
    if rendered.chars().all(|ch| ch.is_ascii_digit()) {
        return format!("#{rendered}");
    }
    rendered
}

fn parse_slack_link(value: &str) -> Option<(&str, &str)> {
    let inner = value.strip_prefix('<')?.strip_suffix('>')?;
    inner.split_once('|')
}

fn render_slack_mrkdwn_line(line: &str) -> String {
    let line = strip_heading_marker(line);
    let line = convert_markdown_links(line);
    convert_markdown_bold(&line)
}

fn strip_heading_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return line;
    }
    let mut hash_count = 0usize;
    let mut rest_start = None;
    for (index, ch) in trimmed.char_indices() {
        if ch == '#' {
            hash_count += 1;
            continue;
        }
        rest_start = Some(index);
        break;
    }
    if !(1..=6).contains(&hash_count) {
        return line;
    }
    let Some(rest_start) = rest_start else {
        return line;
    };
    let rest = &trimmed[rest_start..];
    let Some(rest) = rest.strip_prefix(' ') else {
        return line;
    };
    rest
}

fn convert_markdown_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut index = 0;
    while index < line.len() {
        if !line.is_char_boundary(index) {
            break;
        }
        let Some((_, ch)) = line[index..].char_indices().next() else {
            break;
        };
        if ch == '['
            && let Some((next_index, label, url)) = markdown_link_at(line, index)
            && is_safe_slack_link_url(url)
        {
            out.push('<');
            out.push_str(url);
            out.push('|');
            out.push_str(label);
            out.push('>');
            index = next_index;
            continue;
        }
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn markdown_link_at(line: &str, start: usize) -> Option<(usize, &str, &str)> {
    if !line.is_char_boundary(start) {
        return None;
    }
    if line[start..].char_indices().next()?.1 != '[' {
        return None;
    }
    let label_start = start + '['.len_utf8();
    let label_end = line[label_start..]
        .char_indices()
        .find_map(|(index, ch)| (ch == ']').then_some(label_start + index))?;
    let after_label = &line[label_end..];
    if !after_label.starts_with("](") {
        return None;
    }
    let url_start = label_end + "](".len();
    let url_end = line[url_start..]
        .char_indices()
        .find_map(|(index, ch)| (ch == ')').then_some(url_start + index))?;
    let next_index = url_end + ')'.len_utf8();
    Some((
        next_index,
        &line[label_start..label_end],
        &line[url_start..url_end],
    ))
}

fn is_safe_slack_link_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn convert_markdown_bold(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for (index, part) in line.split("**").enumerate() {
        if index > 0 {
            out.push('*');
        }
        out.push_str(part);
    }
    out
}

fn is_markdown_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_markdown_table_separator(line: &str) -> bool {
    if !is_markdown_table_row(line) {
        return false;
    }
    line.trim().trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
    })
}

fn render_table_row(line: &str) -> String {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| render_slack_mrkdwn_line(cell.trim()))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_markdown_renders_as_slack_mrkdwn() {
        let text = render_slack_mrkdwn(
            "Here are your top Notion docs:\n\n### Top Priority Docs\n\n1. **NEAR AI Engineering Weekly Updates** ([link](https://www.notion.com/p/abc))\n   - \"Multi tenancy for migrating from Railway => top priority\"\n\n| Doc | Highlight |\n|---|---|\n| **Priority Agents** | Priority Agents |",
        );

        assert!(text.contains("Top Priority Docs"));
        assert!(!text.contains("###"));
        assert!(text.contains("*NEAR AI Engineering Weekly Updates*"));
        assert!(text.contains("<https://www.notion.com/p/abc|link>"));
        assert!(!text.contains("|---|---|"));
        assert!(text.contains("• *Doc:* *Priority Agents*"));
        assert!(text.contains("*Highlight:* Priority Agents"));
    }

    #[test]
    fn unicode_markdown_renders_without_byte_boundary_slicing() {
        let text = render_slack_mrkdwn("### Résumé\nПривет [世界](https://example.com/路径)");

        assert!(text.contains("Résumé"));
        assert!(!text.contains("###"));
        assert!(text.contains("Привет <https://example.com/路径|世界>"));
    }

    #[test]
    fn issue_table_renders_as_slack_bullets_instead_of_raw_pipes() {
        let text = render_slack_mrkdwn(
            "GitHub check result\n\n# | Title | Labels | Assignees | Updated | Status / summary\n[4657](https://github.com/nearai/ironclaw/issues/4657) | **Unify reusable Google OAuth credentials** | enhancement, reborn | serrrfirat | 2026-06-10 | Investigate cross-extension OAuth credential reuse.\n4625 | Slack channel-routed personal and team agents | - | serrrfirat | 2026-06-09 | Mostly implemented.",
        );

        assert!(!text.contains("# | Title | Labels"));
        assert!(text.contains(
            "• <https://github.com/nearai/ironclaw/issues/4657|#4657> *Unify reusable Google OAuth credentials*"
        ));
        assert!(text.contains("  *Labels:* enhancement, reborn"));
        assert!(text.contains("  *Assignees:* serrrfirat"));
        assert!(text.contains("  *Updated:* 2026-06-10"));
        assert!(text.contains("  *Summary:* Investigate cross-extension OAuth credential reuse."));
        assert!(text.contains("• #4625 Slack channel-routed personal and team agents"));
    }

    #[test]
    fn long_text_renders_numbered_chunks_under_soft_limit() {
        let chunks = slack_text_chunks(&"a".repeat(SLACK_TEXT_SOFT_LIMIT_CHARS + 10));

        assert_eq!(chunks.len(), 2);
        for (index, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.chars().count() <= SLACK_TEXT_SOFT_LIMIT_CHARS,
                "chunk {} exceeded soft Slack text limit",
                index + 1
            );
        }
        assert!(chunks[0].starts_with("Part 1/2\n"));
        assert!(chunks[1].starts_with("Part 2/2\n"));
    }
}
