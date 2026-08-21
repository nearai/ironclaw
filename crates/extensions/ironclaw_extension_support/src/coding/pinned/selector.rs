//! Read-tool selector grammar, ported from the pinned upstream sources
//! `packages/coding-agent/src/tools/read-selector.ts` and
//! `packages/coding-agent/src/tools/path-utils.ts` (parseLineRangeChunk /
//! parseLineRanges) at commit `08819b279cf02ae2545e69dad7111ab48d91d35e`.
//!
//! Must match `golden/selectors.json` EXACTLY for all 29 cases (see the
//! harness bin `tests/reborn_coding_engines.rs`).

/// An inclusive line range; `end_line` is `None` for open-ended ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineRange {
    pub(crate) start_line: u64,
    pub(crate) end_line: Option<u64>,
    pub(crate) open_ended: bool,
}

/// Parsed representation of a path-embedded selector (mirrors
/// `ParsedSelector` in the pinned `read-selector.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedSelector {
    None,
    Raw,
    Conflicts,
    Lines { ranges: Vec<LineRange>, raw: bool },
}

impl ParsedSelector {
    /// True when the selector requested verbatim/raw output (alone or
    /// combined with a range) — `isRawSelector` in the pinned source.
    pub(crate) fn is_raw(&self) -> bool {
        matches!(self, ParsedSelector::Raw)
            || matches!(self, ParsedSelector::Lines { raw: true, .. })
    }

    /// True when the selector requested multiple line ranges.
    pub(crate) fn is_multi_range(&self) -> bool {
        matches!(self, ParsedSelector::Lines { ranges, .. } if ranges.len() > 1)
    }
}

/// Render the pinned invalid-selector error for `sel`.
pub(crate) fn invalid_selector_message(sel: &str) -> String {
    format!(
        "Invalid selector ':{sel}'. Use :N, :N-M, :N+K, :N- (open-ended), a comma-separated list of ranges, :raw, or a range combined with raw (e.g. :raw:50-100)."
    )
}

/// `parseSel` from the pinned `read-selector.ts`. Returns the exact pinned
/// error text on invalid selectors.
pub(crate) fn parse_sel(sel: Option<&str>) -> Result<ParsedSelector, String> {
    let Some(sel) = sel else {
        return Ok(ParsedSelector::None);
    };
    if sel.is_empty() {
        return Ok(ParsedSelector::None);
    }

    if sel.contains(':') {
        let chunks: Vec<&str> = sel.split(':').collect();
        if chunks.len() == 2 {
            let a = chunks[0];
            let b = chunks[1];
            let a_is_raw = a.eq_ignore_ascii_case("raw");
            let b_is_raw = b.eq_ignore_ascii_case("raw");
            let range_chunk = if a_is_raw {
                b
            } else if b_is_raw {
                a
            } else {
                ""
            };
            let raw_chunk = if a_is_raw {
                a
            } else if b_is_raw {
                b
            } else {
                ""
            };
            if !range_chunk.is_empty() && !raw_chunk.is_empty() {
                match parse_line_ranges(range_chunk) {
                    Ok(Some(ranges)) => return Ok(ParsedSelector::Lines { ranges, raw: true }),
                    Ok(None) => {}
                    Err(message) => return Err(message),
                }
            }
        }
        if chunks
            .iter()
            .all(|chunk| selector_chunk_looks_read_like(chunk))
        {
            return Err(invalid_selector_message(sel));
        }
        // Unrecognized compound — fall through (sqlite/archive/url consume
        // their own colon syntax in the pinned source; those readers are
        // later slices).
        return Ok(ParsedSelector::None);
    }

    if sel.eq_ignore_ascii_case("raw") {
        return Ok(ParsedSelector::Raw);
    }
    if sel.eq_ignore_ascii_case("conflicts") {
        return Ok(ParsedSelector::Conflicts);
    }
    match parse_line_ranges(sel) {
        Ok(Some(ranges)) => return Ok(ParsedSelector::Lines { ranges, raw: false }),
        Ok(None) => {}
        Err(message) => return Err(message),
    }
    Ok(ParsedSelector::None)
}

fn selector_chunk_looks_read_like(chunk: &str) -> bool {
    let lower = chunk.to_ascii_lowercase();
    if lower == "raw" || lower == "conflicts" {
        return true;
    }
    // /^-\d+(?:[-+]\d+)?$/ — a negative-anchored chunk counts as read-like.
    let negative_anchored = chunk.strip_prefix('-').is_some_and(|rest| {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let remainder = &rest[digits.len()..];
        !digits.is_empty()
            && (remainder.is_empty()
                || remainder.strip_prefix(['-', '+']).is_some_and(|tail| {
                    !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit())
                }))
    });
    if negative_anchored {
        return true;
    }
    matches!(parse_line_ranges(chunk), Ok(Some(_)))
}

/// Inclusive line range parsing of a single `N`, `N-M`, `N-`, `N+K`, or
/// `..`-aliased chunk — `parseLineRangeChunk` in the pinned source. Returns
/// the exact pinned error text on invalid bounds.
fn parse_line_range_chunk(chunk: &str) -> Result<Option<LineRange>, String> {
    // /^L?(\d+)(?:(\.\.|[-+])L?(\d+)?)?$/i
    let mut rest = chunk;
    if rest.starts_with(['L', 'l']) {
        rest = &rest[1..];
    }
    let start_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if start_digits.is_empty() {
        return Ok(None);
    }
    let raw_start: u64 = start_digits.parse().map_err(|error| {
        tracing::debug!(%error, selector = chunk, "pinned selector line number overflow");
        "invalid line number"
    })?;
    rest = &rest[start_digits.len()..];

    let mut sep = "";
    let mut raw_end: Option<u64> = None;
    if !rest.is_empty() {
        if rest.starts_with("..") {
            sep = "..";
            rest = &rest[2..];
        } else if rest.starts_with(['-', '+']) {
            sep = &rest[..1];
            rest = &rest[1..];
        } else {
            return Ok(None);
        }
        if !rest.is_empty() {
            if rest.starts_with(['L', 'l']) {
                rest = &rest[1..];
            }
            let rhs_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if rhs_digits.is_empty() {
                // `N-` / `N..` with no rhs is the open-ended form; a `+`
                // with no rhs fails the regex.
                if sep == "-" || sep == ".." {
                    raw_end = None;
                    rest = "";
                } else {
                    return Ok(None);
                }
            } else {
                raw_end = Some(rhs_digits.parse().map_err(|error| {
                    tracing::debug!(%error, selector = chunk, "pinned selector endpoint overflow");
                    "invalid line number"
                })?);
                rest = &rest[rhs_digits.len()..];
            }
        }
    }
    if !rest.is_empty() {
        return Ok(None);
    }

    if raw_start < 1 {
        return Err("Line selector 0 is invalid; lines are 1-indexed. Use :1.".to_string());
    }
    // `..` is a forgiving alias for `-` (e.g. `2724..2727` == `2724-2727`).
    let canonical_sep = if sep == ".." { "-" } else { sep };
    let mut raw_end_line: Option<u64> = None;
    if canonical_sep == "+" {
        let count = raw_end.unwrap_or(0);
        if count < 1 {
            return Err(format!(
                "Invalid range {raw_start}+{count}: count must be >= 1."
            ));
        }
        raw_end_line = Some(
            raw_start
                .checked_add(count - 1)
                .ok_or("invalid line number")?,
        );
    } else if canonical_sep == "-"
        && let Some(end) = raw_end
    {
        if end < raw_start {
            return Err(format!(
                "Invalid range {raw_start}-{end}: end must be >= start."
            ));
        }
        raw_end_line = Some(end);
    }
    Ok(Some(LineRange {
        start_line: raw_start,
        end_line: raw_end_line,
        open_ended: canonical_sep == "-" && raw_end.is_none(),
    }))
}

/// Parse a comma-separated list of line ranges (e.g. `5-16,960-973`),
/// sorted ascending with overlapping/adjacent ranges merged — `parseLineRanges`
/// in the pinned source. `Ok(None)` mirrors the pinned `null` for strings
/// that are not a range list at all; `Err` carries the exact pinned error
/// text for invalid bounds (thrown `ToolError` in the pinned source).
pub(crate) fn parse_line_ranges(sel: &str) -> Result<Option<Vec<LineRange>>, String> {
    let chunks: Vec<&str> = sel.split(',').collect();
    let mut parsed: Vec<LineRange> = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let Some(range) = parse_line_range_chunk(chunk)? else {
            return Ok(None);
        };
        parsed.push(range);
    }
    if parsed.is_empty() {
        return Ok(None);
    }
    parsed.sort_by_key(|range| range.start_line);

    let mut merged: Vec<LineRange> = vec![parsed[0]];
    for current in parsed.into_iter().skip(1) {
        // `merged` starts with `parsed[0]` and only grows, so `last_mut` is
        // always Some; if it ever were not, push and continue instead of
        // panicking.
        let Some(last) = merged.last_mut() else {
            merged.push(current);
            continue;
        };
        // Open-ended means "to EOF" — any later range is absorbed.
        if last.open_ended {
            continue;
        }
        let last_end = last.end_line.unwrap_or(last.start_line);
        if current.start_line <= last_end.saturating_add(1) {
            if current.open_ended {
                last.open_ended = true;
                last.end_line = None;
            } else if current.end_line.unwrap_or(current.start_line) > last_end {
                last.end_line = current.end_line;
            }
            continue;
        }
        merged.push(current);
    }
    Ok(Some(merged))
}

/// `selToOffsetLimit` from the pinned `read-selector.ts`: the FIRST range
/// only (multi-range callers must branch on `is_multi_range` first).
pub(crate) fn sel_to_offset_limit(parsed: &ParsedSelector) -> (Option<u64>, Option<u64>) {
    if let ParsedSelector::Lines { ranges, .. } = parsed
        && let Some(first) = ranges.first()
    {
        let limit = first.end_line.and_then(|end| {
            end.checked_sub(first.start_line)
                .and_then(|span| span.checked_add(1))
        });
        return (Some(first.start_line), limit);
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_selectors_all_29_cases_parse_exactly() {
        // Case ids from manifest.json selector_case_ids, in order.
        #[allow(clippy::type_complexity)]
        let cases: &[(
            &str,
            Option<(&str, bool)>,
            Option<u64>,
            Option<u64>,
            Option<&str>,
        )] = &[
            ("", Some(("none", false)), None, None, None),
            ("", Some(("none", false)), None, None, None),
            ("50", Some(("lines", false)), Some(50), None, None),
            ("50-", Some(("lines", false)), Some(50), None, None),
            ("50-200", Some(("lines", false)), Some(50), Some(151), None),
            ("50+150", Some(("lines", false)), Some(50), Some(150), None),
            (
                "5-16,960-973",
                Some(("lines", false)),
                Some(5),
                Some(12),
                None,
            ),
            ("5-16,1-3", Some(("lines", false)), Some(1), Some(3), None),
            ("1-10,5-20", Some(("lines", false)), Some(1), Some(20), None),
            ("5-10,11-12", Some(("lines", false)), Some(5), Some(8), None),
            ("10-20,1-5", Some(("lines", false)), Some(1), Some(5), None),
            ("5-20,10-", Some(("lines", false)), Some(5), None, None),
            ("1..10", Some(("lines", false)), Some(1), Some(10), None),
            ("1..", Some(("lines", false)), Some(1), None, None),
            ("L5-L10", Some(("lines", false)), Some(5), Some(6), None),
            ("raw", Some(("raw", false)), None, None, None),
            ("RAW", Some(("raw", false)), None, None, None),
            ("conflicts", Some(("conflicts", false)), None, None, None),
            (
                "raw:50-100",
                Some(("lines", true)),
                Some(50),
                Some(51),
                None,
            ),
            (
                "50-100:raw",
                Some(("lines", true)),
                Some(50),
                Some(51),
                None,
            ),
            ("2-4:raw", Some(("lines", true)), Some(2), Some(3), None),
            ("abc", Some(("none", false)), None, None, None),
            (
                "0",
                None,
                None,
                None,
                Some("Line selector 0 is invalid; lines are 1-indexed. Use :1."),
            ),
            (
                "50+0",
                None,
                None,
                None,
                Some("Invalid range 50+0: count must be >= 1."),
            ),
            (
                "200-100",
                None,
                None,
                None,
                Some("Invalid range 200-100: end must be >= start."),
            ),
            (
                "raw:raw",
                None,
                None,
                None,
                Some(
                    "Invalid selector ':raw:raw'. Use :N, :N-M, :N+K, :N- (open-ended), a comma-separated list of ranges, :raw, or a range combined with raw (e.g. :raw:50-100).",
                ),
            ),
            (
                "conflicts:1-1",
                None,
                None,
                None,
                Some(
                    "Invalid selector ':conflicts:1-1'. Use :N, :N-M, :N+K, :N- (open-ended), a comma-separated list of ranges, :raw, or a range combined with raw (e.g. :raw:50-100).",
                ),
            ),
            ("-1", Some(("none", false)), None, None, None),
            (
                "50-100,200",
                Some(("lines", false)),
                Some(50),
                Some(51),
                None,
            ),
        ];

        for (sel, expected, offset, limit, error) in cases {
            match parse_sel(Some(sel)) {
                Ok(parsed) => {
                    let (kind, raw) = expected.expect("expected parse success");
                    match kind {
                        "none" => assert_eq!(parsed, ParsedSelector::None, "sel {sel:?}"),
                        "raw" => assert_eq!(parsed, ParsedSelector::Raw, "sel {sel:?}"),
                        "conflicts" => assert_eq!(parsed, ParsedSelector::Conflicts, "sel {sel:?}"),
                        "lines" => match &parsed {
                            ParsedSelector::Lines { raw: is_raw, .. } => {
                                assert_eq!(*is_raw, raw, "sel {sel:?}")
                            }
                            other => panic!("sel {sel:?} parsed as {other:?}"),
                        },
                        _ => unreachable!(),
                    }
                    let (actual_offset, actual_limit) = sel_to_offset_limit(&parsed);
                    assert_eq!(actual_offset, *offset, "sel {sel:?} offset");
                    assert_eq!(actual_limit, *limit, "sel {sel:?} limit");
                    assert_eq!(*error, None, "sel {sel:?} expected error");
                }
                Err(message) => {
                    assert_eq!(Some(message.as_str()), *error, "sel {sel:?}");
                }
            }
        }
    }

    #[test]
    fn overlapping_ranges_merge_in_one_forward_pass() {
        // 1-10,5-20 -> 1-20; 5-10,11-12 -> 5-12 (adjacent merge).
        let merged = parse_line_ranges("1-10,5-20")
            .expect("parses")
            .expect("range list");
        assert_eq!(
            merged,
            vec![LineRange {
                start_line: 1,
                end_line: Some(20),
                open_ended: false,
            }]
        );
        let merged = parse_line_ranges("5-10,11-12")
            .expect("parses")
            .expect("range list");
        assert_eq!(
            merged,
            vec![LineRange {
                start_line: 5,
                end_line: Some(12),
                open_ended: false,
            }]
        );
        // 5-20,10- -> open-ended absorbs everything after.
        let merged = parse_line_ranges("5-20,10-")
            .expect("parses")
            .expect("range list");
        assert_eq!(
            merged,
            vec![LineRange {
                start_line: 5,
                end_line: None,
                open_ended: true,
            }]
        );
    }

    #[test]
    fn invalid_selector_message_renders_sel() {
        assert_eq!(
            invalid_selector_message("raw:raw"),
            "Invalid selector ':raw:raw'. Use :N, :N-M, :N+K, :N- (open-ended), a comma-separated list of ranges, :raw, or a range combined with raw (e.g. :raw:50-100)."
        );
    }
}
