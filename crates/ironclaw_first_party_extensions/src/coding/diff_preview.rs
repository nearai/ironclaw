use ironclaw_host_api::{CapabilityDisplayOutputPreview, truncate_capability_display_text};
use similar::{ChangeTag, TextDiff};

const DIFF_CONTEXT_LINES: usize = 3;
const DIFF_PREVIEW_DETAILED_INPUT_MAX_BYTES: usize = 256 * 1024;
const DIFF_PREVIEW_MAX_BYTES: usize = 16 * 1024;

/// Returns `true` when `new_content` is large enough that a detailed diff
/// preview will be skipped regardless of the old content. Callers (e.g.
/// `write_file`) can use this to avoid reading the old file unnecessarily.
pub(super) fn will_use_large_diff_path(new_content: &str) -> bool {
    new_content.len() > DIFF_PREVIEW_DETAILED_INPUT_MAX_BYTES / 2
}

pub(super) fn file_diff_preview(
    path: &str,
    old_content: &str,
    new_content: &str,
) -> CapabilityDisplayOutputPreview {
    let diff_path = path.trim_start_matches('/');
    if old_content.len().saturating_add(new_content.len()) > DIFF_PREVIEW_DETAILED_INPUT_MAX_BYTES {
        return large_diff_preview(path, diff_path);
    }

    let diff = TextDiff::from_lines(old_content, new_content);
    let mut output = String::new();
    output.push_str(&format!("--- a/{diff_path}\n"));
    output.push_str(&format!("+++ b/{diff_path}\n"));

    // Accumulate addition/deletion counts and build output in a single grouped_ops pass.
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut truncated = false;
    for group in diff.grouped_ops(DIFF_CONTEXT_LINES) {
        if output.len() >= DIFF_PREVIEW_MAX_BYTES {
            truncated = true;
            break;
        }
        let Some(first_op) = group.first() else {
            continue;
        };
        let Some(last_op) = group.last() else {
            continue;
        };
        let old_start = hunk_start(first_op.old_range().start, first_op.old_range().len());
        let new_start = hunk_start(first_op.new_range().start, first_op.new_range().len());
        let old_len = last_op.old_range().end - first_op.old_range().start;
        let new_len = last_op.new_range().end - first_op.new_range().start;
        output.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_len, new_start, new_len
        ));
        for op in group {
            for change in diff.iter_changes(&op) {
                let prefix = match change.tag() {
                    ChangeTag::Delete => {
                        deletions += 1;
                        '-'
                    }
                    ChangeTag::Insert => {
                        additions += 1;
                        '+'
                    }
                    ChangeTag::Equal => ' ',
                };
                push_diff_line(&mut output, prefix, change.value());
                if output.len() >= DIFF_PREVIEW_MAX_BYTES {
                    truncated = true;
                    break;
                }
            }
            if truncated {
                break;
            }
        }
    }

    let preview_text = truncate_capability_display_text(&output, DIFF_PREVIEW_MAX_BYTES);
    CapabilityDisplayOutputPreview {
        output_summary: Some(format!("Edited 1 file: +{additions}/-{deletions}")),
        output_preview: preview_text.text,
        output_kind: "unified_diff".to_string(),
        subtitle: Some(path.to_string()),
        truncated: truncated || preview_text.truncated,
    }
}

fn large_diff_preview(path: &str, diff_path: &str) -> CapabilityDisplayOutputPreview {
    CapabilityDisplayOutputPreview {
        output_summary: Some("Edited 1 file (large diff preview omitted)".to_string()),
        output_preview: format!(
            "--- a/{diff_path}\n+++ b/{diff_path}\n@@ large diff preview omitted @@\n"
        ),
        output_kind: "unified_diff".to_string(),
        subtitle: Some(path.to_string()),
        truncated: true,
    }
}

fn hunk_start(start: usize, len: usize) -> usize {
    if len == 0 { start } else { start + 1 }
}

fn push_diff_line(output: &mut String, prefix: char, value: &str) {
    output.push(prefix);
    output.push_str(value);
    if !value.ends_with('\n') {
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::{DIFF_PREVIEW_MAX_BYTES, file_diff_preview};

    #[test]
    fn file_diff_preview_emits_unified_diff_and_stats() {
        let preview = file_diff_preview(
            "src/main.rs",
            "fn main() {\n    old();\n}\n",
            "fn main() {\n    new();\n}\n",
        );

        assert_eq!(preview.output_kind, "unified_diff");
        assert_eq!(
            preview.output_summary.as_deref(),
            Some("Edited 1 file: +1/-1")
        );
        assert!(
            preview
                .output_preview
                .contains("--- a/src/main.rs\n+++ b/src/main.rs\n@@")
        );
        assert!(preview.output_preview.contains("-    old();"));
        assert!(preview.output_preview.contains("+    new();"));
    }

    #[test]
    fn file_diff_preview_emits_multiple_hunks_without_rewriting_middle_context() {
        let old_content = "one\nold-a\nthree\nfour\nfive\nsix\nold-b\neight\n";
        let new_content = "one\nnew-a\nthree\nfour\nfive\nsix\nnew-b\neight\n";

        let preview = file_diff_preview("src/main.rs", old_content, new_content);

        assert_eq!(
            preview.output_summary.as_deref(),
            Some("Edited 1 file: +2/-2")
        );
        assert!(preview.output_preview.contains("-old-a\n+new-a"));
        assert!(preview.output_preview.contains("-old-b\n+new-b"));
        assert!(
            !preview
                .output_preview
                .contains("-three\n-four\n-five\n-six")
        );
    }

    #[test]
    fn file_diff_preview_truncates_on_utf8_boundary() {
        let old_content = (0..2000)
            .map(|index| format!("old-{index}-é\n"))
            .collect::<String>();
        let new_content = (0..2000)
            .map(|index| format!("new-{index}-é\n"))
            .collect::<String>();

        let preview = file_diff_preview("src/main.rs", &old_content, &new_content);

        assert!(preview.truncated);
        assert!(
            preview
                .output_preview
                .is_char_boundary(preview.output_preview.len())
        );
        assert!(preview.output_preview.len() <= DIFF_PREVIEW_MAX_BYTES);
    }

    #[test]
    fn file_diff_preview_zero_change_returns_plus0_minus0_no_hunk_header() {
        let content = "fn main() {}\n";
        let preview = file_diff_preview("src/lib.rs", content, content);

        assert_eq!(preview.output_kind, "unified_diff");
        assert_eq!(
            preview.output_summary.as_deref(),
            Some("Edited 1 file: +0/-0")
        );
        // No @@ hunk header: grouped_ops yields no groups for identical content.
        assert!(!preview.output_preview.contains("@@"));
        assert!(!preview.truncated);
    }

    #[test]
    fn file_diff_preview_omits_detailed_diff_for_large_inputs() {
        let old_content = "old\n".repeat(80 * 1024);
        let new_content = "new\n".repeat(80 * 1024);

        let preview = file_diff_preview("src/main.rs", &old_content, &new_content);

        assert!(preview.truncated);
        assert_eq!(
            preview.output_summary.as_deref(),
            Some("Edited 1 file (large diff preview omitted)")
        );
        assert!(
            preview
                .output_preview
                .contains("large diff preview omitted")
        );
    }
}
