//! Document redline / tracked-changes — paragraph-level diff between two
//! documents in a project. v1 ships an HTML output (`<ins>`/`<del>` tags);
//! a follow-up will turn the same diff into DOCX revision marks via
//! `docx-rs`.
//!
//! The diff splits each document on blank lines (so paragraphs are the
//! granularity), runs longest-common-subsequence over the paragraph
//! sequences, and emits a sequence of `Equal`/`Removed`/`Added` ops. We
//! deliberately don't go to word- or character-level granularity in v1;
//! paragraph-level is the standard mike redline shape and avoids the
//! Myers-diff implementation work for the first cut.

use serde::Serialize;

/// One step in the diff. The granularity is paragraph-level: each
/// `Equal` carries one paragraph that's identical between base and
/// candidate, each `Removed` is a paragraph that disappeared, each
/// `Added` is a paragraph that appeared.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DiffOp {
    Equal { text: String },
    Removed { text: String },
    Added { text: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct RedlineStats {
    pub paragraphs_total: usize,
    pub unchanged: usize,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedlineResult {
    pub ops: Vec<DiffOp>,
    pub html: String,
    pub stats: RedlineStats,
}

/// Compute a paragraph-level diff between `base` and `candidate` text.
///
/// Both strings are split on blank lines and trimmed; consecutive empty
/// paragraphs are dropped so a stray formatting whitespace doesn't show
/// up as an addition or removal.
pub fn redline(base: &str, candidate: &str) -> RedlineResult {
    let a = paragraphs(base);
    let b = paragraphs(candidate);
    let lcs = lcs_table(&a, &b);
    let ops = backtrack(&a, &b, &lcs);

    let (mut unchanged, mut added, mut removed) = (0usize, 0usize, 0usize);
    for op in &ops {
        match op {
            DiffOp::Equal { .. } => unchanged += 1,
            DiffOp::Added { .. } => added += 1,
            DiffOp::Removed { .. } => removed += 1,
        }
    }

    let html = render_html(&ops);
    RedlineResult {
        ops,
        html,
        stats: RedlineStats {
            paragraphs_total: a.len().max(b.len()),
            unchanged,
            added,
            removed,
        },
    }
}

/// Split on blank-line boundaries. Each paragraph is trimmed; runs of
/// internal whitespace are preserved verbatim. Empty results are dropped
/// so two empty paragraphs in a row don't generate spurious ops.
fn paragraphs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut blank_run = false;
    for line in s.split('\n') {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if !blank_run && !current.is_empty() {
                out.push(std::mem::take(&mut current).trim().to_string());
            }
            blank_run = true;
            current.clear();
        } else {
            blank_run = false;
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(trimmed);
        }
    }
    if !current.is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|p| !p.is_empty());
    out
}

/// Standard O(n·m) LCS table. Document text is already capped at the
/// per-doc budget elsewhere, so the quadratic memory is fine for v1. If
/// a real workload pushes past a few thousand paragraphs per side we
/// can swap in Hunt-Szymanski or Myers later.
fn lcs_table(a: &[String], b: &[String]) -> Vec<Vec<usize>> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    dp
}

/// Walk back through the LCS table to produce the diff. Order of ops
/// in the result matches the natural reading order of `candidate` with
/// removals interleaved at their original position.
fn backtrack(a: &[String], b: &[String], dp: &[Vec<usize>]) -> Vec<DiffOp> {
    let mut i = a.len();
    let mut j = b.len();
    let mut ops = Vec::with_capacity(i + j);
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            ops.push(DiffOp::Equal {
                text: a[i - 1].clone(),
            });
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            ops.push(DiffOp::Removed {
                text: a[i - 1].clone(),
            });
            i -= 1;
        } else {
            ops.push(DiffOp::Added {
                text: b[j - 1].clone(),
            });
            j -= 1;
        }
    }
    while i > 0 {
        ops.push(DiffOp::Removed {
            text: a[i - 1].clone(),
        });
        i -= 1;
    }
    while j > 0 {
        ops.push(DiffOp::Added {
            text: b[j - 1].clone(),
        });
        j -= 1;
    }
    ops.reverse();
    ops
}

/// Render the diff as HTML. Each paragraph is wrapped in a `<p>`; added
/// paragraphs are wrapped in `<ins>`, removed in `<del>`. Content is
/// HTML-escaped before insertion so a hostile document body can't
/// inject markup into the rendered output.
fn render_html(ops: &[DiffOp]) -> String {
    let mut out = String::with_capacity(ops.len() * 64);
    for op in ops {
        match op {
            DiffOp::Equal { text } => {
                out.push_str("<p>");
                out.push_str(&escape_html(text));
                out.push_str("</p>\n");
            }
            DiffOp::Added { text } => {
                out.push_str("<p><ins>");
                out.push_str(&escape_html(text));
                out.push_str("</ins></p>\n");
            }
            DiffOp::Removed { text } => {
                out.push_str("<p><del>");
                out.push_str(&escape_html(text));
                out.push_str("</del></p>\n");
            }
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_split_on_blank_lines() {
        let input = "first\nline\n\nsecond\n\n\nthird\n";
        assert_eq!(
            paragraphs(input),
            vec![
                "first\nline".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }

    #[test]
    fn equal_documents_produce_no_changes() {
        let r = redline("a\n\nb\n\nc", "a\n\nb\n\nc");
        assert_eq!(r.stats.unchanged, 3);
        assert_eq!(r.stats.added, 0);
        assert_eq!(r.stats.removed, 0);
    }

    #[test]
    fn pure_addition_only_marks_new_paragraphs() {
        let base = "a\n\nb";
        let cand = "a\n\nb\n\nc";
        let r = redline(base, cand);
        assert_eq!(r.stats.unchanged, 2);
        assert_eq!(r.stats.added, 1);
        assert_eq!(r.stats.removed, 0);
        assert!(r.html.contains("<ins>c</ins>"));
    }

    #[test]
    fn pure_removal_only_marks_dropped_paragraphs() {
        let base = "a\n\nb\n\nc";
        let cand = "a\n\nc";
        let r = redline(base, cand);
        assert_eq!(r.stats.unchanged, 2);
        assert_eq!(r.stats.added, 0);
        assert_eq!(r.stats.removed, 1);
        assert!(r.html.contains("<del>b</del>"));
    }

    #[test]
    fn replacement_is_one_remove_plus_one_add() {
        let base = "intro\n\nold paragraph\n\nclose";
        let cand = "intro\n\nnew paragraph\n\nclose";
        let r = redline(base, cand);
        assert_eq!(r.stats.unchanged, 2);
        assert_eq!(r.stats.added, 1);
        assert_eq!(r.stats.removed, 1);
        assert!(r.html.contains("<del>old paragraph</del>"));
        assert!(r.html.contains("<ins>new paragraph</ins>"));
    }

    #[test]
    fn html_escapes_special_chars_in_text() {
        let base = "hello";
        let cand = "<script>alert(1)</script>";
        let r = redline(base, cand);
        assert!(r.html.contains("&lt;script&gt;"));
        assert!(!r.html.contains("<script>"));
    }
}
