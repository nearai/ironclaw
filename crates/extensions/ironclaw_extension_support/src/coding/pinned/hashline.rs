//! Hashline edit engine, ported from the pinned upstream sources
//! (`packages/hashline/src/*` and `packages/coding-agent/src/edit/hashline/*`
//! plus `edit/index.ts` aggregate shapes) at commit
//! `08819b279cf02ae2545e69dad7111ab48d91d35e`.
//!
//! Implements the model-visible hashline contract: `[path#TAG]` section
//! headers, `PUT`/`CUT`/`REM`/`MV` ops, `N.=M` ranges, `+TEXT` body rows,
//! `@registers`, `<1`/`>N`/`>$` gaps, `N*` block suffixes, snapshot-anchored
//! stale-anchor rejection with the exact pinned messages, and CAS writes via
//! [`RootFilesystem::put`] with [`CasExpectation::Version`].
//!
//! Deliberate scope decisions (documented; see also the slice spec):
//! - No fuzzy recovery: a drifted file is NEVER edited; the exact pinned
//!   MismatchError message is rendered instead (hash recognized vs not from
//!   this session).
//! - Block locators resolve via a lexical brace-matching resolver (the
//!   pinned upstream uses a tree-sitter native resolver; tree-sitter is a
//!   later-slice dependency). Unresolvable anchors produce the exact
//!   pinned `blockUnresolvedMessage` text.
//! - The apply-time heuristic repairs (replacement boundary echoes,
//!   indentation auto-shift, landing correction) are not ported; they only
//!   alter output for off-by-one payloads and never change the pinned
//!   error/output shapes this slice pins.
//! - The `edit.enforceSeenLines` guard is not ported (it needs the
//!   read-tool's displayed-line provenance, a later-slice concern).
//! - Same-path section merging does not flag interleaved clipboard reorder
//!   (the pinned `CLIPBOARD_INTERLEAVED_SECTIONS` guard).

use ironclaw_filesystem::{CasExpectation, Entry, FilesystemOperation};
use serde_json::Value;

use super::state::{CodingScopeKey, CodingSnapshotRegistry};
use super::{
    CodingEngineContext, CodingEngineError, CodingEngineErrorKind, coding_error, input_error,
    resolve_input_path,
};

macro_rules! hl_const {
    ($($part:expr),+ $(,)?) => {
        concat!($($part),+)
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// format — pinned `packages/hashline/src/format.ts`
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) mod format {
    /// File-section header delimiters: `[path#hash]`.
    pub(crate) const HL_FILE_PREFIX: &str = "[";
    pub(crate) const HL_FILE_SUFFIX: &str = "]";
    /// Payload sigil for literal body rows.
    pub(crate) const HL_PAYLOAD_REPLACE: &str = "+";
    /// Hunk-header keywords.
    pub(crate) const HL_PUT_KEYWORD: &str = "PUT";
    pub(crate) const HL_CUT_KEYWORD: &str = "CUT";
    pub(crate) const HL_REM_KEYWORD: &str = "REM";
    pub(crate) const HL_MOVE_KEYWORD: &str = "MV";
    #[allow(dead_code)]
    pub(crate) const HL_HEADER_COLON: &str = ":";
    /// Gap sigils.
    #[allow(dead_code)]
    pub(crate) const HL_GAP_BEFORE: &str = "<";
    #[allow(dead_code)]
    pub(crate) const HL_GAP_AFTER: &str = ">";
    /// Locator suffix: `N*` extends the anchor to the syntactic block.
    #[allow(dead_code)]
    pub(crate) const HL_BLOCK_SUFFIX: &str = "*";
    /// Gap anchor: `$` names the last line, so `>$` is end-of-file.
    #[allow(dead_code)]
    pub(crate) const HL_EOF_ANCHOR: &str = "$";
    /// Register sigil: `@name`.
    #[allow(dead_code)]
    pub(crate) const HL_REGISTER_SIGIL: &str = "@";
    /// Separator between a hashline file path and its opaque snapshot tag.
    pub(crate) const HL_FILE_HASH_SEP: &str = "#";
    /// Canonical separator between inclusive range endpoints, e.g. `5.=10`.
    pub(crate) const HL_RANGE_SEP: &str = ".=";
    /// Separator between a line number and displayed line content.
    pub(crate) const HL_LINE_BODY_SEP: &str = ":";

    /// Number of hex characters in a content-derived file-hash tag.
    pub(crate) const HL_FILE_HASH_LENGTH: usize = 4;
    /// Representative file-hash tags for user-facing error messages.
    pub(crate) const HL_FILE_HASH_EXAMPLES: [&str; 3] = ["1A2B", "3C4D", "9F3E"];

    /// Normalize text before hashing: trim trailing `[ \t\r]` from every
    /// line (and the final line) in a single pass so CRLF endings and
    /// display-trimmed lines do not invalidate a tag.
    pub(crate) fn normalize_file_hash_text(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let bytes = text.as_bytes();
        let mut line_start = 0usize;
        let mut i = 0usize;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\n' || i + 1 == bytes.len() {
                // Line content spans [line_start, end). On a trailing `\n`
                // the newline itself is excluded (re-emitted below); on the
                // final line (no trailing newline) the last byte is included
                // so it is not dropped before the whitespace trim.
                let mut end = if b == b'\n' { i } else { i + 1 };
                while end > line_start {
                    match bytes[end - 1] {
                        b' ' | b'\t' | b'\r' => end -= 1,
                        _ => break,
                    }
                }
                out.push_str(&text[line_start..end]);
                if b == b'\n' {
                    out.push('\n');
                }
                line_start = i + 1;
            }
            i += 1;
        }
        out
    }

    /// Compute the content-derived hash tag: 4-hex uppercase xxHash32 of the
    /// normalized file text, masked to 16 bits (`computeFileHash` in the
    /// pinned source; `Bun.hash.xxHash32(normalized, 0) & 0xffff`).
    pub(crate) fn compute_file_hash(text: &str) -> String {
        let normalized = normalize_file_hash_text(text);
        let low16 = xxhash_rust::xxh32::xxh32(normalized.as_bytes(), 0) & 0xffff;
        format!("{low16:04X}")
    }

    /// Format a hashline section header for a file path and snapshot tag.
    pub(crate) fn format_hashline_header(file_path: &str, file_hash: &str) -> String {
        format!("{HL_FILE_PREFIX}{file_path}{HL_FILE_HASH_SEP}{file_hash}{HL_FILE_SUFFIX}")
    }

    /// Format a single numbered line as `LINE:TEXT`.
    pub(crate) fn format_numbered_line(line_number: u64, line: &str) -> String {
        format!("{line_number}{HL_LINE_BODY_SEP}{line}")
    }

    /// Format file text with hashline-mode line-number prefixes.
    pub(crate) fn format_numbered_lines(text: &str, start_line: u64) -> String {
        text.split('\n')
            .enumerate()
            .map(|(index, line)| format_numbered_line(start_line + index as u64, line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

use format::*;

// ═══════════════════════════════════════════════════════════════════════════
// messages — pinned `packages/hashline/src/messages.ts`
// ═══════════════════════════════════════════════════════════════════════════

/// Lines of context shown either side of a hash mismatch.
pub(crate) const MISMATCH_CONTEXT: u64 = 2;

/// Numbered `LINE:TEXT` rows around `anchor_lines` (±MISMATCH_CONTEXT),
/// `*`-marking anchors, `...` between non-adjacent runs.
pub(crate) fn format_anchored_context(anchor_lines: &[u64], file_lines: &[String]) -> Vec<String> {
    let mut display_lines: Vec<u64> = Vec::new();
    for &line in anchor_lines {
        if line < 1 || line > file_lines.len() as u64 {
            continue;
        }
        let lo = 1u64.max(line.saturating_sub(MISMATCH_CONTEXT));
        let hi = (file_lines.len() as u64).min(line + MISMATCH_CONTEXT);
        for line_num in lo..=hi {
            display_lines.push(line_num);
        }
    }
    display_lines.sort_unstable();
    display_lines.dedup();

    let mut rows: Vec<String> = Vec::new();
    let mut previous: Option<u64> = None;
    for &line_num in &display_lines {
        if let Some(previous) = previous
            && line_num > previous + 1
        {
            rows.push("...".to_string());
        }
        let marker = if anchor_lines.contains(&line_num) {
            "*"
        } else {
            " "
        };
        let text = file_lines
            .get((line_num - 1) as usize)
            .map(String::as_str)
            .unwrap_or("");
        rows.push(format!("{marker}{}", format_numbered_line(line_num, text)));
        previous = Some(line_num);
    }
    rows
}

/// Format the required-shape diagnostic shown when a line reference is
/// malformed (`formatFullAnchorRequirement` in the pinned source).
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn format_full_anchor_requirement(raw: Option<&str>) -> String {
    let received = match raw {
        Some(raw) => format!(
            " Received {}.",
            serde_json::to_string(raw).unwrap_or_default()
        ),
        None => String::new(),
    };
    format!(
        "a bare line number from read/search output plus the section header content-hash tag (for example {HL_FILE_PREFIX}src/foo.ts{HL_FILE_HASH_SEP}{}{HL_FILE_SUFFIX} and line \"160\"){received}",
        HL_FILE_HASH_EXAMPLES[0]
    )
}

/// `invalidAbsoluteRangeMessage` in the pinned source.
pub(crate) fn invalid_absolute_range_message(
    patch_line: u64,
    start: u64,
    end: u64,
    op: AbsoluteRangeOp,
    block: Option<(u64, u64)>,
    register: Option<&str>,
) -> String {
    let single = match op {
        AbsoluteRangeOp::Replace => match register {
            Some(reg) => format!("PUT {start} @{reg}"),
            None => format!("PUT {start}:"),
        },
        AbsoluteRangeOp::Cut => match register {
            Some(reg) => format!("CUT {start} @{reg}"),
            None => format!("CUT {start}"),
        },
    };
    let counted_end = start + end - 1;
    let counted = if counted_end >= start {
        let range = match op {
            AbsoluteRangeOp::Replace => match register {
                Some(reg) => format!("PUT {start}{HL_RANGE_SEP}{counted_end} @{reg}"),
                None => format!("PUT {start}{HL_RANGE_SEP}{counted_end}:"),
            },
            AbsoluteRangeOp::Cut => match register {
                Some(reg) => format!("CUT {start}{HL_RANGE_SEP}{counted_end} @{reg}"),
                None => format!("CUT {start}{HL_RANGE_SEP}{counted_end}"),
            },
        };
        Some(range)
    } else {
        None
    };
    let block_form = match op {
        AbsoluteRangeOp::Replace => match register {
            Some(reg) => format!("PUT {start}* @{reg}"),
            None => format!("PUT {start}*:"),
        },
        AbsoluteRangeOp::Cut => match register {
            Some(reg) => format!("CUT {start}* @{reg}"),
            None => format!("CUT {start}*"),
        },
    };
    let mut message = format!(
        "line {patch_line}: Invalid absolute range: start {start}, end {end}. \
         The value after `{HL_RANGE_SEP}` is an absolute source line, not a line count or replacement length. \
         For one line use `{single}`."
    );
    if let Some(counted) = counted {
        message.push_str(&format!(
            " For {end} lines starting at {start}, use `{counted}`."
        ));
    }
    if let Some((block_start, block_end)) = block
        && block_start == start
        && block_end > start
    {
        message.push_str(&format!(
            " The syntactic block beginning at {start} ends at {block_end}, so `{block_form}` is also valid."
        ));
    }
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbsoluteRangeOp {
    Replace,
    Cut,
}

/// Exact-range duplicate hunks were normalized to the final hunk.
pub(crate) const REPLACE_PAIR_COALESCED_WARNING: &str = "Multiple hunks targeted the same exact range; kept only the last. Issue one `PUT` or `CUT` hunk per range.";

/// Bare body rows auto-converted to literal `+` rows.
pub(crate) const BARE_BODY_AUTO_PIPED_WARNING: &str =
    "Auto-prefixed bare body row(s) with `+`. Body rows must be `+TEXT` literal lines.";

/// Copied read-output elision rows were ignored rather than written as source.
pub(crate) const READ_METADATA_IGNORED_WARNING: &str =
    "Ignored copied read-output elision row(s). Re-read elided ranges before editing them.";

/// Empty span/block PUT recovered as a delete-only edit.
pub(crate) const EMPTY_PUT_AUTO_CUT_WARNING: &str = hl_const!(
    "Interpreted an empty `PUT` body as deletion. Use `CUT N",
    ".=",
    "M` or `CUT N*` for bodyless deletes."
);

/// A bodyless CUT carried a harmless trailing colon.
pub(crate) const CUT_COLON_IGNORED_WARNING: &str = hl_const!(
    "Ignored a trailing `:` on bodyless `CUT`. Prefer `CUT N",
    ".=",
    "M` / `CUT N*` without a colon."
);

/// `-` rows are not valid in a hunk body.
pub(crate) const MINUS_ROW_REJECTED: &str = "`-` rows are not valid; the range already names the lines being changed. For Markdown bullets or other literal `-` lines, prefix the literal row with `+`: `+- item`.";

/// Unified-diff old rows were discarded; explicit `+` rows are final content.
pub(crate) const DIFF_OLD_ROWS_IGNORED_WARNING: &str = "Ignored unified-diff `-old` row(s); the range already removes old content, so only `+new` rows were kept.";

/// Bare `-` body rows accepted as literal Markdown bullets.
pub(crate) const MINUS_BULLET_AUTO_PIPED_WARNING: &str = "Auto-prefixed bare `- ` bullet row(s) as literal content. `-` rows never remove lines — the range does that; always prefix literal body rows with `+`: `+- item`.";

/// Register `PUT` header carried a `:`.
pub(crate) const COLON_ON_REGISTER_PUT: &str = "`PUT … @name` pastes the register and never takes `:` — the colon promises body rows. Drop the colon (`PUT >40 @name`), or drop `@name` and write `+TEXT` body rows.";

/// Register `PUT` hunk received a body row.
pub(crate) const REGISTER_PUT_TAKES_NO_BODY: &str = "A register `PUT` pastes captured lines and takes no `+` body rows. To write literal text, drop the `@name` and use `PUT …:` with body rows.";

/// Colonless `PUT` hunk received a body row.
pub(crate) const COLONLESS_PUT_TAKES_NO_BODY: &str = "`PUT` without `:` is clipboard-backed and takes no body rows. Add `:` after the locator to write literal content (`PUT >40:` then `+TEXT` rows).";

/// Colonless anonymous `PUT` on a span target.
pub(crate) const COLONLESS_SPAN_PUT: &str = hl_const!(
    "Colonless `PUT` is clipboard-backed, and span targets need a named register (`PUT 5",
    ".=",
    "9 @name`); the anonymous register pastes only at gaps (`PUT >40`). To write literal content, add `:` and `+TEXT` body rows."
);

/// Anonymous paste ran with an empty anonymous register.
pub(crate) const EMPTY_PASTE: &str = hl_const!(
    "Nothing to paste: no unlabeled `CUT` precedes this `PUT` in this call, and the anonymous register never carries across calls. Put `CUT N",
    ".=",
    "M` / `CUT N*` above it, or use named registers (`CUT … @name` → `PUT … @name`) for cross-call moves."
);

/// Gap `PUT` with `:` but no body.
pub(crate) const EMPTY_INSERT: &str = "`PUT <N:` / `PUT >N:` promises body rows and got none. Write `+TEXT` rows, or drop the `:` to paste a register (`PUT >N` = anonymous, `PUT >N @name` = named).";

/// `CUT` hunk received a body row.
pub(crate) const CUT_TAKES_NO_BODY: &str = hl_const!(
    "`CUT` deletes (and captures) the named lines and takes no body rows. To write new content, use `PUT N",
    ".=",
    "M:` with `+TEXT` rows."
);

/// `REM` received a body row or coexists with line edits.
pub(crate) const REM_TAKES_NO_BODY: &str = "`REM` deletes the whole file and takes no body rows or line ops. Issue it alone under the header.";

/// `MV` received a body row.
pub(crate) const MOVE_TAKES_NO_BODY: &str = "`MV DEST` does not take body rows. Put line edits above the `MV` row; the destination path follows `MV` on the same line.";

/// `PUT >N*:` anchored on a closing-delimiter line; applied as `PUT >N:`.
pub(crate) fn insert_after_block_closer_lowered_warning(line: u64) -> String {
    format!(
        "`PUT >{line}*:` anchors on a closing delimiter, so it was applied as plain `PUT >{line}:`. Anchor on the line that OPENS the construct."
    )
}

/// `PUT >N*:` anchor unresolvable; applied as `PUT >N:`.
pub(crate) fn insert_after_block_unresolved_lowered_warning(line: u64) -> String {
    format!(
        "`PUT >{line}*:` could not resolve a syntactic block on line {line}, so it was applied as plain `PUT >{line}:`. Verify the landing line; anchor on a line that OPENS a construct."
    )
}

/// Register `PUT >N*` anchored on a closing-delimiter line; applied as `PUT >N`.
pub(crate) fn paste_after_block_closer_lowered_warning(line: u64) -> String {
    format!(
        "`PUT >{line}*` anchors on a closing delimiter, so it was applied as plain `PUT >{line}`. Anchor on the line that OPENS the construct."
    )
}

/// Register `PUT >N*` anchor unresolvable; applied as `PUT >N`.
pub(crate) fn paste_after_block_unresolved_lowered_warning(line: u64) -> String {
    format!(
        "`PUT >{line}*` could not resolve a syntactic block on line {line}, so it was applied as plain `PUT >{line}`. Verify the landing line; anchor on a line that OPENS a construct."
    )
}

/// Applied the `PUT <1:`/`PUT >$:` edit despite a stale snapshot tag.
pub(crate) const HEADTAIL_DRIFT_WARNING: &str = "Applied the `PUT <1:`/`PUT >$:` edit despite a stale snapshot tag (file changed since your read) — head/tail position is content-independent. Re-read if the drift was unexpected.";

/// Section omitted the mandatory snapshot tag.
pub(crate) fn missing_snapshot_tag_message(section_path: &str) -> String {
    format!(
        "Missing hashline snapshot tag for {section_path}; use `{HL_FILE_PREFIX}{section_path}{HL_FILE_HASH_SEP}tag{HL_FILE_SUFFIX}` from your latest read/search output. To create a new file, use the write tool."
    )
}

/// A section named a path that does not exist, but its filename and snapshot
/// tag matched a file read earlier this session.
#[allow(dead_code)]
pub(crate) fn path_recovered_from_tag_message(
    authored_path: &str,
    resolved_path: &str,
    tag: &str,
) -> String {
    format!(
        "Path \"{authored_path}\" does not exist; matched its filename and snapshot tag {HL_FILE_HASH_SEP}{tag} to {resolved_path} (read earlier this session). Anchor future edits on {HL_FILE_PREFIX}{resolved_path}{HL_FILE_HASH_SEP}TAG{HL_FILE_SUFFIX}."
    )
}

/// Unlabeled paste with two or more unlabeled cuts pending.
pub(crate) fn ambiguous_anonymous_paste_message(pending: &[String]) -> String {
    format!(
        "{} unlabeled `CUT`s are pending ({}) — an unlabeled paste cannot tell which one you meant. Label the moves (`CUT … @name` → `PUT … @name`), or keep at most one unlabeled `CUT` before each unlabeled paste.",
        pending.len(),
        pending.join(", ")
    )
}

/// Named paste read a register that holds nothing.
pub(crate) fn empty_register_paste_warning(name: &str, known: &[String]) -> String {
    let base = format!(
        "`@{name}` was empty — no `CUT … @{name}` precedes this op in this call and no persisted register has that name — so nothing was pasted (a span target is still removed)."
    );
    if known.is_empty() {
        base
    } else {
        let registers = known
            .iter()
            .map(|key| format!("`@{key}`"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{base} Available registers: {registers}.")
    }
}

/// Clipboard ops inside a same-path section merged across another file's section.
#[allow(dead_code)]
pub(crate) const CLIPBOARD_INTERLEAVED_SECTIONS: &str = "`CUT`/register-`PUT` ops cannot be used in a file whose sections are interleaved with another file's: same-path sections merge into the first occurrence, which would reorder the register sequence. Keep each file's ops under ONE `[path#TAG]` header.";

/// Block-anchored edit reached a path with no block resolver.
#[allow(dead_code)]
pub(crate) const BLOCK_RESOLVER_UNAVAILABLE: &str = "Block locators (`N*` in `PUT N*:`, `PUT >N*`, `CUT N*`) are not available here (no block resolver configured). Use a concrete line range.";

#[derive(Default)]
pub(crate) struct BlockDiagnosticSuggestions {
    pub(crate) next_block: Option<(u64, u64)>,
    pub(crate) enclosing_block: Option<(u64, u64)>,
}

fn block_form_at(op: AbsoluteRangeOp, line: u64, register: Option<&str>) -> String {
    match op {
        AbsoluteRangeOp::Replace => match register {
            Some(reg) => format!("PUT {line}* @{reg}"),
            None => format!("PUT {line}*:"),
        },
        AbsoluteRangeOp::Cut => match register {
            Some(reg) => format!("CUT {line}* @{reg}"),
            None => format!("CUT {line}*"),
        },
    }
}

/// A block-anchored replace/cut could not resolve to a syntactic block.
pub(crate) fn block_unresolved_message(
    line: u64,
    op: AbsoluteRangeOp,
    file_lines: Option<&[String]>,
    suggestions: &BlockDiagnosticSuggestions,
    register: Option<&str>,
) -> String {
    let phrase = block_form_at(op, line, register);
    let fallback = match op {
        AbsoluteRangeOp::Replace => match register {
            Some(reg) => format!("PUT {line}{HL_RANGE_SEP}M @{reg}"),
            None => format!("PUT {line}{HL_RANGE_SEP}M:"),
        },
        AbsoluteRangeOp::Cut => match register {
            Some(reg) => format!("CUT {line}{HL_RANGE_SEP}M @{reg}"),
            None => format!("CUT {line}{HL_RANGE_SEP}M"),
        },
    };
    let anchor_text = file_lines.and_then(|lines| lines.get((line - 1) as usize));
    let mut message = if let Some(anchor_text) = anchor_text {
        match suggestions.next_block {
            Some((next_start, next_end)) if anchor_text.trim().is_empty() => {
                let retry = block_form_at(op, next_start, register);
                format!(
                    "Line {line} is blank; no syntactic block can begin there. The next multi-line block begins at line {next_start} and ends at line {next_end}. Retry `{retry}`."
                )
            }
            _ => format!(
                "`{phrase}` could not resolve a syntactic block beginning on line {line} (unsupported language, blank/closer line, or parse error). Use `{fallback}` with explicit lines."
            ),
        }
    } else {
        format!(
            "`{phrase}` could not resolve a syntactic block beginning on line {line} (unsupported language, blank/closer line, or parse error). Use `{fallback}` with explicit lines."
        )
    };
    if let Some((enclosing_start, enclosing_end)) = suggestions.enclosing_block {
        let retry = block_form_at(op, enclosing_start, register);
        message.push_str(&format!(
            " The nearest enclosing multi-line block begins at line {enclosing_start} and ends at line {enclosing_end}; use `{retry}` to target it."
        ));
    }
    if let Some(lines) = file_lines {
        let context = format_anchored_context(&[line], lines);
        if !context.is_empty() {
            message.push_str("\n\n");
            message.push_str(&context.join("\n"));
        }
    }
    message
}

/// A block-op anchor resolved to a single line (bare statement, not the
/// opening line of a multi-line construct).
pub(crate) fn block_single_line_message(
    line: u64,
    op: BlockOp,
    enclosing_block: Option<(u64, u64)>,
) -> String {
    let plain = match op {
        BlockOp::Replace => format!("PUT {line}:"),
        BlockOp::InsertAfter => format!("PUT >{line}:"),
        BlockOp::Cut => format!("CUT {line}"),
        BlockOp::PasteAfter => format!("PUT >{line}"),
    };
    let mut message = format!(
        "`{}` resolved to a single line: line {line} is a bare statement, not the opening line of a multi-line construct. Use `{plain}` for a single line.",
        block_op_form(op, line)
    );
    if let Some((start, end)) = enclosing_block {
        message.push_str(&format!(
            " The nearest enclosing multi-line block begins at line {start} and ends at line {end}; use `{}` to target it.",
            block_op_form(op, start)
        ));
    }
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockOp {
    Replace,
    InsertAfter,
    Cut,
    PasteAfter,
}

fn block_op_form(op: BlockOp, line: u64) -> String {
    match op {
        BlockOp::Replace => format!("PUT {line}*:"),
        BlockOp::InsertAfter => format!("PUT >{line}*:"),
        BlockOp::Cut => format!("CUT {line}*"),
        BlockOp::PasteAfter => format!("PUT >{line}*"),
    }
}

/// Thrown by `executeHashlineSingle` when the parsed patch has zero sections.
pub(crate) const NO_HASHLINE_SECTIONS: &str = "No hashline sections found in input.";

// ═══════════════════════════════════════════════════════════════════════════
// mismatch — pinned `packages/hashline/src/mismatch.ts`
// ═══════════════════════════════════════════════════════════════════════════

/// Render the exact stale-anchor rejection (`MismatchError.formatMessage` in
/// the pinned source). `hash_recognized` selects the recognized vs
/// not-from-session wording; `anchor_lines` feeds the anchored-context
/// preview.
pub(crate) fn render_mismatch_message(
    path: Option<&str>,
    expected_file_hash: &str,
    actual_file_hash: &str,
    file_lines: &[String],
    anchor_lines: &[u64],
    hash_recognized: bool,
) -> String {
    let path_text = path.map(|path| format!(" for {path}")).unwrap_or_default();
    let mut lines: Vec<String> = if !hash_recognized {
        vec![
            format!(
                "Edit rejected{path_text}: hash {HL_FILE_HASH_SEP}{expected_file_hash} is not from this session."
            ),
            format!(
                "The current file hashes to {HL_FILE_HASH_SEP}{actual_file_hash}. Re-read the file with `read` to copy a current {HL_FILE_PREFIX}path{HL_FILE_HASH_SEP}tag{HL_FILE_SUFFIX} header — never invent the tag and never reuse one from a prior session."
            ),
        ]
    } else {
        vec![
            format!("Edit rejected{path_text}: file changed between read and edit."),
            format!(
                "Section is bound to {HL_FILE_HASH_SEP}{expected_file_hash}, but the current file hashes to {HL_FILE_HASH_SEP}{actual_file_hash}. If a prior edit in this session modified this file, copy the {HL_FILE_PREFIX}path{HL_FILE_HASH_SEP}newhash{HL_FILE_SUFFIX} header from that edit's response; otherwise re-read the file with `read` to refresh the tag before retrying."
            ),
        ]
    };
    let context = format_anchored_context(anchor_lines, file_lines);
    if context.is_empty() {
        lines.join("\n")
    } else {
        lines.push(String::new());
        lines.extend(context);
        lines.join("\n")
    }
}

/// `parseTag`'s malformed line-reference message. The received segment
/// rendered by `format_full_anchor_requirement` already carries its own
/// trailing period, so no extra one is appended here.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn malformed_line_reference(raw: &str) -> String {
    format!(
        "Invalid line reference. Expected {}",
        format_full_anchor_requirement(Some(raw))
    )
}

/// `validateLineRef`'s out-of-bounds message.
pub(crate) fn line_out_of_bounds(line: u64, line_count: usize) -> String {
    format!("Line {line} does not exist (file has {line_count} lines)")
}

// ═══════════════════════════════════════════════════════════════════════════
// data types — pinned `packages/hashline/src/types.ts` (subset)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub(crate) line: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cursor {
    Bof,
    Eof,
    BeforeAnchor(Anchor),
    AfterAnchor(Anchor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertMode {
    Replacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockMode {
    InsertAfter,
    Cut,
    PasteAfter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedRange {
    pub(crate) start: Anchor,
    pub(crate) end: Anchor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PasteTarget {
    Gap(Cursor),
    Span(ParsedRange),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Edit {
    Insert {
        cursor: Cursor,
        text: String,
        line_num: u64,
        index: usize,
        mode: Option<InsertMode>,
        block_start: Option<u64>,
    },
    Delete {
        anchor: Anchor,
        line_num: u64,
        index: usize,
    },
    Cut {
        range: ParsedRange,
        register: Option<String>,
        line_num: u64,
        index: usize,
    },
    Paste {
        at: PasteTarget,
        register: Option<String>,
        line_num: u64,
        index: usize,
        block_start: Option<u64>,
    },
    Block {
        anchor: Anchor,
        payloads: Vec<String>,
        mode: Option<BlockMode>,
        register: Option<String>,
        line_num: u64,
        index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileOp {
    Rem,
    Move { dest: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyResult {
    pub(crate) text: String,
    pub(crate) first_changed_line: Option<u64>,
    pub(crate) warnings: Vec<String>,
    pub(crate) block_resolutions: Vec<BlockResolution>,
}

impl ApplyResult {
    pub(crate) fn noop(text: String) -> Self {
        Self {
            text,
            first_changed_line: None,
            warnings: Vec::new(),
            block_resolutions: Vec::new(),
        }
    }
}

/// One block-op anchor resolved to its concrete line span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockResolution {
    pub(crate) anchor_line: u64,
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) op: BlockOp,
}

// ═══════════════════════════════════════════════════════════════════════════
// tokenizer — pinned `packages/hashline/src/tokenizer.ts`
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BlockTarget {
    Replace {
        range: ParsedRange,
        register: Option<String>,
    },
    Block {
        anchor: Anchor,
        register: Option<String>,
    },
    InsertBefore {
        anchor: Anchor,
        register: Option<String>,
    },
    InsertAfter {
        anchor: Anchor,
        register: Option<String>,
    },
    InsertAfterBlock {
        anchor: Anchor,
        register: Option<String>,
    },
    Cut {
        range: ParsedRange,
        register: Option<String>,
    },
    CutBlock {
        anchor: Anchor,
        register: Option<String>,
    },
    Bof {
        register: Option<String>,
    },
    Eof {
        register: Option<String>,
    },
    Rem,
    Move {
        dest: String,
    },
}

impl BlockTarget {
    pub(crate) fn register(&self) -> Option<&str> {
        match self {
            BlockTarget::Replace { register, .. }
            | BlockTarget::Block { register, .. }
            | BlockTarget::InsertBefore { register, .. }
            | BlockTarget::InsertAfter { register, .. }
            | BlockTarget::InsertAfterBlock { register, .. }
            | BlockTarget::Cut { register, .. }
            | BlockTarget::CutBlock { register, .. }
            | BlockTarget::Bof { register }
            | BlockTarget::Eof { register } => register.as_deref(),
            BlockTarget::Rem | BlockTarget::Move { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token {
    Blank {
        line_num: u64,
    },
    EnvelopeBegin {
        line_num: u64,
    },
    EnvelopeEnd {
        line_num: u64,
    },
    Abort {
        line_num: u64,
    },
    Header {
        line_num: u64,
        path: String,
        file_hash: Option<String>,
    },
    OpBlock {
        line_num: u64,
        target: BlockTarget,
        had_colon: bool,
    },
    PayloadLiteral {
        line_num: u64,
        text: String,
    },
    Raw {
        line_num: u64,
        text: String,
    },
}

const BEGIN_PATCH_MARKER: &str = "*** Begin Patch";
const END_PATCH_MARKER: &str = "*** End Patch";
const ABORT_MARKER: &str = "*** Abort";

fn marker_line_equals(line: &str, marker: &str) -> bool {
    line.trim() == marker
}

fn is_whitespace_code(code: u8) -> bool {
    code == b' ' || (b'\t'..=b'\r').contains(&code)
}

fn skip_whitespace(line: &[u8], mut index: usize, end: usize) -> usize {
    while index < end && is_whitespace_code(line[index]) {
        index += 1;
    }
    index
}

fn trim_end_index(line: &str) -> usize {
    line.trim_end().len()
}

fn is_nonzero_digit_code(code: u8) -> bool {
    code.is_ascii_digit() && code != b'0'
}

fn is_digit_code(code: u8) -> bool {
    code.is_ascii_digit()
}

fn scan_line_number(line: &[u8], index: usize, end: usize) -> Option<(u64, usize)> {
    if index >= end || !is_nonzero_digit_code(line[index]) {
        return None;
    }
    let mut line_number: u64 = 0;
    let mut next_index = index;
    while next_index < end {
        let code = line[next_index];
        if !is_digit_code(code) {
            break;
        }
        line_number = line_number * 10 + u64::from(code - b'0');
        next_index += 1;
    }
    Some((line_number, next_index))
}

/// Range separator scanner: canonical `.=`, lenient `-`, `=`, `.`, `..`,
/// `…`, mixed runs, whitespace-only.
fn scan_range_separator(line: &[u8], index: usize, end: usize) -> Option<usize> {
    let mut cursor = index;
    let mut consumed = false;
    while cursor < end {
        let code = line[cursor];
        if is_whitespace_code(code) || code == b'-' || code == b'.' || code == b'=' {
            cursor += 1;
            consumed = true;
            continue;
        }
        // U+2026 horizontal ellipsis (multi-byte).
        if line[cursor..].starts_with("\u{2026}".as_bytes()) {
            cursor += "\u{2026}".len();
            consumed = true;
            continue;
        }
        break;
    }
    if !consumed || cursor >= end || !is_nonzero_digit_code(line[cursor]) {
        return None;
    }
    Some(cursor)
}

struct RangeScan {
    range: ParsedRange,
    next_index: usize,
    had_separator: bool,
}

fn scan_header_range(
    line: &[u8],
    index: usize,
    end: usize,
    allow_single: bool,
) -> Option<RangeScan> {
    let number_start = skip_whitespace(line, index, end);
    let (start_line, start_next) = scan_line_number(line, number_start, end)?;
    match scan_range_separator(line, start_next, end) {
        None => {
            if !allow_single {
                return None;
            }
            let next = skip_whitespace(line, start_next, end);
            Some(RangeScan {
                range: ParsedRange {
                    start: Anchor { line: start_line },
                    end: Anchor { line: start_line },
                },
                next_index: next,
                had_separator: false,
            })
        }
        Some(after_first) => {
            let (end_line, end_next) = scan_line_number(line, after_first, end)?;
            Some(RangeScan {
                range: ParsedRange {
                    start: Anchor { line: start_line },
                    end: Anchor { line: end_line },
                },
                next_index: skip_whitespace(line, end_next, end),
                had_separator: true,
            })
        }
    }
}

fn scan_keyword(line: &[u8], index: usize, end: usize, keyword: &str) -> Option<usize> {
    if !line[index..end].starts_with(keyword.as_bytes()) {
        return None;
    }
    let next = index + keyword.len();
    if next < end {
        let code = line[next];
        if !is_whitespace_code(code) && code != b':' {
            return None;
        }
    }
    Some(next)
}

fn consume_optional_colon(line: &[u8], index: usize, end: usize) -> (usize, bool) {
    let cursor = skip_whitespace(line, index, end);
    if cursor < end && line[cursor] == b':' {
        (skip_whitespace(line, cursor + 1, end), true)
    } else {
        (cursor, false)
    }
}

fn is_register_name_code(code: u8) -> bool {
    code.is_ascii_digit()
        || code.is_ascii_uppercase()
        || code.is_ascii_lowercase()
        || code == b'_'
        || code == b'-'
}

const REGISTER_NAME_MAX: usize = 64;

fn scan_register(line: &[u8], index: usize, end: usize) -> Option<(String, usize)> {
    if index >= end || line[index] != b'@' {
        return None;
    }
    let start = index + 1;
    let mut cursor = start;
    while cursor < end && is_register_name_code(line[cursor]) {
        cursor += 1;
    }
    if cursor == start || cursor - start > REGISTER_NAME_MAX {
        return None;
    }
    Some((
        String::from_utf8_lossy(&line[start..cursor]).into_owned(),
        cursor,
    ))
}

fn finish_target_scan(
    line: &[u8],
    index: usize,
    end: usize,
    target: BlockTarget,
) -> Option<(BlockTarget, bool)> {
    let cursor = skip_whitespace(line, index, end);
    let mut target = target;
    if let Some((name, next)) = scan_register(line, cursor, end) {
        target = with_register(target, name);
        let (after, had_colon) = consume_optional_colon(line, next, end);
        if after != end {
            return None;
        }
        return Some((target, had_colon));
    }
    let (after, had_colon) = consume_optional_colon(line, cursor, end);
    if after != end {
        return None;
    }
    Some((target, had_colon))
}

fn with_register(target: BlockTarget, register: String) -> BlockTarget {
    match target {
        BlockTarget::Replace { range, .. } => BlockTarget::Replace {
            range,
            register: Some(register),
        },
        BlockTarget::Block { anchor, .. } => BlockTarget::Block {
            anchor,
            register: Some(register),
        },
        BlockTarget::InsertBefore { anchor, .. } => BlockTarget::InsertBefore {
            anchor,
            register: Some(register),
        },
        BlockTarget::InsertAfter { anchor, .. } => BlockTarget::InsertAfter {
            anchor,
            register: Some(register),
        },
        BlockTarget::InsertAfterBlock { anchor, .. } => BlockTarget::InsertAfterBlock {
            anchor,
            register: Some(register),
        },
        BlockTarget::Cut { range, .. } => BlockTarget::Cut {
            range,
            register: Some(register),
        },
        BlockTarget::CutBlock { anchor, .. } => BlockTarget::CutBlock {
            anchor,
            register: Some(register),
        },
        BlockTarget::Bof { .. } => BlockTarget::Bof {
            register: Some(register),
        },
        BlockTarget::Eof { .. } => BlockTarget::Eof {
            register: Some(register),
        },
        other => other,
    }
}

fn scan_put_target(line: &[u8], index: usize, end: usize) -> Option<(BlockTarget, bool)> {
    let cursor = skip_whitespace(line, index, end);
    if cursor >= end {
        return None;
    }
    let sigil = line[cursor];
    if sigil == b'<' || sigil == b'>' {
        let is_after = sigil == b'>';
        let probe = skip_whitespace(line, cursor + 1, end);
        if is_after && probe < end && line[probe] == b'$' {
            return finish_target_scan(line, probe + 1, end, BlockTarget::Eof { register: None });
        }
        let (anchor_line, mut next) = scan_line_number(line, probe, end)?;
        let mut block = false;
        if next < end && line[next] == b'*' {
            block = true;
            next += 1;
        }
        if is_after {
            return finish_target_scan(
                line,
                next,
                end,
                if block {
                    BlockTarget::InsertAfterBlock {
                        anchor: Anchor { line: anchor_line },
                        register: None,
                    }
                } else {
                    BlockTarget::InsertAfter {
                        anchor: Anchor { line: anchor_line },
                        register: None,
                    }
                },
            );
        }
        // `<N*` is the same gap as `<N`; `<1` is head -> bof.
        return finish_target_scan(
            line,
            next,
            end,
            if anchor_line == 1 {
                BlockTarget::Bof { register: None }
            } else {
                BlockTarget::InsertBefore {
                    anchor: Anchor { line: anchor_line },
                    register: None,
                }
            },
        );
    }
    let range = scan_header_range(line, cursor, end, true)?;
    let next = range.next_index;
    if next < end && line[next] == b'*' {
        if range.had_separator {
            return None;
        }
        return finish_target_scan(
            line,
            next + 1,
            end,
            BlockTarget::Block {
                anchor: Anchor {
                    line: range.range.start.line,
                },
                register: None,
            },
        );
    }
    finish_target_scan(
        line,
        next,
        end,
        BlockTarget::Replace {
            range: range.range,
            register: None,
        },
    )
}

fn scan_cut_target(line: &[u8], index: usize, end: usize) -> Option<(BlockTarget, bool)> {
    let range = scan_header_range(line, index, end, true)?;
    let next = range.next_index;
    if next < end && line[next] == b'*' {
        if range.had_separator {
            return None;
        }
        return finish_target_scan(
            line,
            next + 1,
            end,
            BlockTarget::CutBlock {
                anchor: Anchor {
                    line: range.range.start.line,
                },
                register: None,
            },
        );
    }
    finish_target_scan(
        line,
        next,
        end,
        BlockTarget::Cut {
            range: range.range,
            register: None,
        },
    )
}

fn unquote_path(path_text: &str) -> String {
    if path_text.len() < 2 {
        return path_text.to_string();
    }
    let bytes = path_text.as_bytes();
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    if (first == b'"' || first == b'\'') && first == last {
        path_text[1..path_text.len() - 1].to_string()
    } else {
        path_text.to_string()
    }
}

fn scan_move_dest(line: &[u8], index: usize, end: usize) -> Option<String> {
    let cursor = skip_whitespace(line, index, end);
    if cursor >= end {
        return None;
    }
    if line[cursor] == b'"' || line[cursor] == b'\'' {
        let quote = line[cursor];
        let mut next = cursor + 1;
        while next < end {
            if line[next] == b'\\' && next + 1 < end {
                next += 2;
                continue;
            }
            if line[next] == quote {
                let after = skip_whitespace(line, next + 1, end);
                if after == end {
                    let raw = String::from_utf8_lossy(&line[cursor..next + 1]).into_owned();
                    return Some(unquote_path(&raw));
                }
                return None;
            }
            next += 1;
        }
        return None;
    }
    let raw = String::from_utf8_lossy(&line[cursor..end]).into_owned();
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(unquote_path(&trimmed))
    }
}

fn scan_hunk_anchor(line: &[u8], start: usize, end: usize) -> Option<(BlockTarget, bool)> {
    let cursor = skip_whitespace(line, start, end);

    if let Some(rem_end) = scan_keyword(line, cursor, end, HL_REM_KEYWORD) {
        if skip_whitespace(line, rem_end, end) != end {
            return None;
        }
        return Some((BlockTarget::Rem, false));
    }
    if let Some(move_end) = scan_keyword(line, cursor, end, HL_MOVE_KEYWORD) {
        let dest = scan_move_dest(line, move_end, end)?;
        return Some((BlockTarget::Move { dest }, false));
    }
    if let Some(put_end) = scan_keyword(line, cursor, end, HL_PUT_KEYWORD) {
        return scan_put_target(line, put_end, end);
    }
    if let Some(cut_end) = scan_keyword(line, cursor, end, HL_CUT_KEYWORD) {
        return scan_cut_target(line, cut_end, end);
    }
    None
}

fn try_parse_hunk_header(line: &str) -> Option<(BlockTarget, bool)> {
    let end = trim_end_index(line);
    let bytes = line.as_bytes();
    let start = skip_whitespace(bytes, 0, end);
    if start >= end {
        return None;
    }
    scan_hunk_anchor(bytes, start, end)
}

fn try_parse_header(line: &str) -> Option<(String, Option<String>)> {
    if !line.starts_with(HL_FILE_PREFIX) {
        return None;
    }
    let end = trim_end_index(line);
    if !line.ends_with(HL_FILE_SUFFIX) {
        return None;
    }
    let body_end = end - HL_FILE_SUFFIX.len();
    if HL_FILE_PREFIX.len() >= body_end {
        return None;
    }
    let bytes = line.as_bytes();
    let mut path_end = body_end;
    let mut file_hash: Option<String> = None;
    let trailing_hash_start = body_end - HL_FILE_HASH_LENGTH - 1;
    if trailing_hash_start >= HL_FILE_PREFIX.len() && bytes[trailing_hash_start] == b'#' {
        let mut all_hex = true;
        for byte in bytes.iter().take(body_end).skip(trailing_hash_start + 1) {
            if !byte.is_ascii_hexdigit() {
                all_hex = false;
                break;
            }
        }
        if all_hex {
            path_end = trailing_hash_start;
            file_hash = Some(
                String::from_utf8_lossy(&bytes[trailing_hash_start + 1..body_end])
                    .to_ascii_uppercase(),
            );
        }
    }
    for byte in &bytes[HL_FILE_PREFIX.len()..path_end] {
        if *byte == b'#' {
            return None;
        }
    }
    if path_end == HL_FILE_PREFIX.len() {
        return None;
    }
    let path = line[HL_FILE_PREFIX.len()..path_end].to_string();
    Some((path, file_hash))
}

/// Classify one line into a token (mirrors `classifyLine`).
fn classify_line(line: &str, line_num: u64) -> Token {
    if line.is_empty() {
        return Token::Blank { line_num };
    }
    if marker_line_equals(line, BEGIN_PATCH_MARKER) {
        return Token::EnvelopeBegin { line_num };
    }
    if marker_line_equals(line, END_PATCH_MARKER) {
        return Token::EnvelopeEnd { line_num };
    }
    if marker_line_equals(line, ABORT_MARKER) {
        return Token::Abort { line_num };
    }
    if line.starts_with(HL_FILE_PREFIX)
        && let Some((path, file_hash)) = try_parse_header(line)
    {
        return Token::Header {
            line_num,
            path,
            file_hash,
        };
    }
    let bytes = line.as_bytes();
    let lead = skip_whitespace(bytes, 0, bytes.len());
    let is_hunk_lead = bytes[lead..].starts_with(HL_PUT_KEYWORD.as_bytes())
        || bytes[lead..].starts_with(HL_CUT_KEYWORD.as_bytes())
        || bytes[lead..].starts_with(HL_REM_KEYWORD.as_bytes())
        || bytes[lead..].starts_with(HL_MOVE_KEYWORD.as_bytes());
    if is_hunk_lead && let Some((target, had_colon)) = try_parse_hunk_header(line) {
        return Token::OpBlock {
            line_num,
            target,
            had_colon,
        };
    }
    if let Some(rest) = line.strip_prefix(HL_PAYLOAD_REPLACE) {
        return Token::PayloadLiteral {
            line_num,
            text: rest.to_string(),
        };
    }
    Token::Raw {
        line_num,
        text: line.to_string(),
    }
}

/// Split hashline text into lines with CRLF stripped.
fn split_hashline_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// prefixes — pinned `packages/hashline/src/prefixes.ts` (subset)
// ═══════════════════════════════════════════════════════════════════════════

static HL_PREFIX_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*(?:>>>|>>)?\s*(?:[+*-]\s*)?\d+[:|]").expect("static prefix regex") // safety: hardcoded compile-time regex literal
});
static HL_PREFIX_PLUS_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*(?:>>>|>>)?\s*\+\s*\d+:").expect("static prefix plus regex") // safety: hardcoded compile-time regex literal
});
static HL_HEADER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*\[[^#\r\n]+#[0-9a-fA-F]{4}\]\s*$").expect("static header regex") // safety: hardcoded compile-time regex literal
});
static DIFF_PLUS_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^[+]([^+]|$)").expect("static diff plus regex") // safety: hardcoded compile-time regex literal
});
static READ_TRUNCATION_NOTICE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(
    || {
        regex::Regex::new(
        r"^\s*\[(?:(?:Showing lines \d+-\d+ of \d+|\d+ more lines? in (?:file|\S+))\b.*\bUse :L?\d+|(?:…|\.\.\.)?\d+\s*ln elided;\s*re-read needed ranges with .+)\]\s*$",
    )
    .expect("static truncation regex") // safety: hardcoded compile-time regex literal
    },
);
static READ_RANGE_ELISION_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*[1-9]\d*\s*-\s*[1-9]\d*:.*(?:…|\.\.\.).*$")
        .expect("static range elision regex") // safety: hardcoded compile-time regex literal
});
static READ_SINGLE_ELISION_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*(?:…|\.\.\.)\s*$").expect("static single elision regex") // safety: hardcoded compile-time regex literal
});

/// Whether a row is display-only metadata emitted by `read`, never source.
pub(crate) fn is_read_metadata_line(line: &str) -> bool {
    READ_TRUNCATION_NOTICE_RE.is_match(line)
        || READ_RANGE_ELISION_RE.is_match(line)
        || READ_SINGLE_ELISION_RE.is_match(line)
}

/// Strip a single leading hashline prefix (`N:`, `N|`, `>>>N:`, `+N:` …).
fn strip_one_leading_hashline_prefix(line: &str) -> String {
    HL_PREFIX_RE.replace(line, "").into_owned()
}

struct LinePrefixStats {
    non_empty: usize,
    header_count: usize,
    hash_prefix_count: usize,
    diff_plus_hash_prefix_count: usize,
    diff_plus_count: usize,
}

fn collect_line_prefix_stats(lines: &[String]) -> LinePrefixStats {
    let mut stats = LinePrefixStats {
        non_empty: 0,
        header_count: 0,
        hash_prefix_count: 0,
        diff_plus_hash_prefix_count: 0,
        diff_plus_count: 0,
    };
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if is_read_metadata_line(line) {
            continue;
        }
        if HL_HEADER_RE.is_match(line) {
            stats.non_empty += 1;
            stats.header_count += 1;
            continue;
        }
        stats.non_empty += 1;
        if HL_PREFIX_RE.is_match(line) {
            stats.hash_prefix_count += 1;
        }
        if HL_PREFIX_PLUS_RE.is_match(line) {
            stats.diff_plus_hash_prefix_count += 1;
        }
        if DIFF_PLUS_RE.is_match(line) {
            stats.diff_plus_count += 1;
        }
    }
    stats
}

/// `stripNewLinePrefixes`: strip whichever prefix scheme the lines appear to
/// be carrying; untouched if no scheme is recognized.
#[allow(dead_code)]
fn strip_new_line_prefixes(lines: &[String]) -> Vec<String> {
    let stats = collect_line_prefix_stats(lines);
    if stats.non_empty == 0 {
        return lines.to_vec();
    }
    let content_line_count = stats.non_empty - stats.header_count;
    let strip_hash = content_line_count > 0 && stats.hash_prefix_count == content_line_count;
    let strip_plus = !strip_hash
        && stats.diff_plus_hash_prefix_count == 0
        && stats.diff_plus_count > 0
        && stats.diff_plus_count as f64 >= stats.non_empty as f64 * 0.5;

    if !strip_hash && !strip_plus && stats.diff_plus_hash_prefix_count == 0 {
        return lines.to_vec();
    }

    lines
        .iter()
        .filter(|line| !(is_read_metadata_line(line) || strip_hash && HL_HEADER_RE.is_match(line)))
        .map(|line| {
            if strip_hash {
                let mut result = line.clone();
                loop {
                    let stripped = HL_PREFIX_RE.replace(&result, "").into_owned();
                    if stripped == result {
                        break;
                    }
                    result = stripped;
                }
                result
            } else if strip_plus {
                DIFF_PLUS_RE.replace(line, "$1").into_owned()
            } else if stats.diff_plus_hash_prefix_count > 0 && HL_PREFIX_PLUS_RE.is_match(line) {
                HL_PREFIX_RE.replace(line, "").into_owned()
            } else {
                line.clone()
            }
        })
        .collect()
}

/// `stripHashlinePrefixes`: strict variant — strip only when every content
/// line is hashline-prefixed.
pub(crate) fn strip_hashline_prefixes(lines: &[String]) -> Vec<String> {
    let stats = collect_line_prefix_stats(lines);
    if stats.non_empty == 0 {
        return lines.to_vec();
    }
    let content_line_count = stats.non_empty - stats.header_count;
    if content_line_count == 0 || stats.hash_prefix_count != content_line_count {
        return lines.to_vec();
    }
    lines
        .iter()
        .filter(|line| !is_read_metadata_line(line) && !HL_HEADER_RE.is_match(line))
        .map(|line| {
            let mut result = line.clone();
            loop {
                let stripped = HL_PREFIX_RE.replace(&result, "").into_owned();
                if stripped == result {
                    break;
                }
                result = stripped;
            }
            result
        })
        .collect()
}

/// `hashlineParseText`: normalize line payloads by stripping read/search
/// line prefixes; a single multiline string is split on `\n`.
#[allow(dead_code)]
pub(crate) fn hashline_parse_text(edit: &str) -> Vec<String> {
    let trimmed = edit.strip_suffix('\n').unwrap_or(edit);
    let lines: Vec<String> = trimmed
        .replace('\r', "")
        .split('\n')
        .map(ToString::to_string)
        .collect();
    strip_new_line_prefixes(&lines)
}

// ═══════════════════════════════════════════════════════════════════════════
// parser — pinned `packages/hashline/src/parser.ts`
// ═══════════════════════════════════════════════════════════════════════════

const MAX_EXPANDED_RANGE_LINES: u64 = 100_000;

fn validate_range(
    range: &ParsedRange,
    line_num: u64,
    op: AbsoluteRangeOp,
    register: Option<&str>,
) -> Result<(), String> {
    if range.start.line < 1 || range.end.line < 1 {
        return Err(format!(
            "line {line_num}: {} range endpoints must be positive safe integers; got {} and {}.",
            op_name(op),
            range.start.line,
            range.end.line
        ));
    }
    if range.end.line < range.start.line {
        return Err(invalid_absolute_range_message(
            line_num,
            range.start.line,
            range.end.line,
            op,
            None,
            register,
        ));
    }
    let span = range.end.line - range.start.line + 1;
    if span > MAX_EXPANDED_RANGE_LINES {
        return Err(format!(
            "line {line_num}: {} range spans {span} lines; the maximum is {MAX_EXPANDED_RANGE_LINES}. Split it into smaller hunks.",
            op_name(op)
        ));
    }
    Ok(())
}

fn op_name(op: AbsoluteRangeOp) -> &'static str {
    match op {
        AbsoluteRangeOp::Replace => "replace",
        AbsoluteRangeOp::Cut => "cut",
    }
}

fn is_skippable_comment_line(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// `bodylessTargetMessage`: body-row rejection for ops that take no rows.
fn bodyless_target_message(target: &BlockTarget, had_colon: bool) -> Option<&'static str> {
    match target {
        BlockTarget::Cut { .. } | BlockTarget::CutBlock { .. } => Some(CUT_TAKES_NO_BODY),
        BlockTarget::Rem | BlockTarget::Move { .. } => None,
        BlockTarget::Replace {
            register: Some(_), ..
        }
        | BlockTarget::Block {
            register: Some(_), ..
        }
        | BlockTarget::InsertBefore {
            register: Some(_), ..
        }
        | BlockTarget::InsertAfter {
            register: Some(_), ..
        }
        | BlockTarget::InsertAfterBlock {
            register: Some(_), ..
        }
        | BlockTarget::Bof { register: Some(_) }
        | BlockTarget::Eof { register: Some(_) } => Some(REGISTER_PUT_TAKES_NO_BODY),
        _ if !had_colon => Some(COLONLESS_PUT_TAKES_NO_BODY),
        _ => None,
    }
}

static TOP_LEVEL_SNAPSHOT_ROW_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^\s*([1-9]\d*)[:|](.*)$").expect("static snapshot row regex") // safety: hardcoded compile-time regex literal
    });
static TOP_LEVEL_BARE_RANGE_HEADER_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^\s*([1-9]\d*)(?:\s|[-.=…])+([1-9]\d*)\s*:\s*$")
            .expect("static bare range regex") // safety: hardcoded compile-time regex literal
    });
static MD_BULLET_ROW_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*- \S").expect("static bullet regex") // safety: hardcoded compile-time regex literal
});
static BARE_LITERAL_VALUE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"^\s*(?:"[^"]*"|'[^']*'|[-+]?\d+(?:\.\d+)?)\s*,?\s*$"#)
        .expect("static literal regex") // safety: hardcoded literal
});

fn parse_top_level_snapshot_row(text: &str) -> Option<(u64, String)> {
    let captures = TOP_LEVEL_SNAPSHOT_ROW_RE.captures(text)?;
    let line: u64 = captures[1].parse().ok()?;
    Some((line, captures[2].to_string()))
}

fn parse_top_level_bare_range_header(text: &str) -> Option<ParsedRange> {
    let captures = TOP_LEVEL_BARE_RANGE_HEADER_RE.captures(text)?;
    let start: u64 = captures[1].parse().ok()?;
    let end: u64 = captures[2].parse().ok()?;
    Some(ParsedRange {
        start: Anchor { line: start },
        end: Anchor { line: end },
    })
}

fn detect_apply_patch_contamination(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("*** Update File:")
        || trimmed.starts_with("*** Add File:")
        || trimmed.starts_with("*** Delete File:")
        || trimmed.starts_with("*** Move to:")
    {
        let preview = if trimmed.len() > 48 {
            format!("{}…", &trimmed[..trimmed.floor_char_boundary(48)])
        } else {
            trimmed.to_string()
        };
        return Some(format!(
            "apply_patch sentinel {} is not valid in hashline. File sections start with `[path#HASH]` (no `Update File:` / `Add File:` keyword). Use `PUT N{HL_RANGE_SEP}M:`, `CUT N{HL_RANGE_SEP}M`, or `PUT <N:`/`PUT >N:` ops.",
            serde_json::to_string(&preview).unwrap_or_default()
        ));
    }
    let unified_header = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^@@\s+[-+]?\d+,\d+\s+[-+]?\d+,\d+\s+@@")
            .expect("static unified header regex") // safety: hardcoded compile-time regex literal
    });
    if unified_header.is_match(trimmed) {
        return Some(format!(
            "unified-diff hunk header (`@@ -N,M +N,M @@`) is not valid in hashline. Use `PUT N{HL_RANGE_SEP}M:`, `CUT N{HL_RANGE_SEP}M`, or `PUT <N:`/`PUT >N:` ops."
        ));
    }
    if trimmed.starts_with("@@") {
        let preview = if trimmed.len() > 48 {
            format!("{}…", &trimmed[..trimmed.floor_char_boundary(48)])
        } else {
            trimmed.to_string()
        };
        return Some(format!(
            "`@@`-bracketed hunk header {} is not valid in hashline. Drop the `@@ ... @@` brackets and write a header such as `PUT N{HL_RANGE_SEP}M:`.",
            serde_json::to_string(&preview).unwrap_or_default()
        ));
    }
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!(
            "hunk headers need a verb and both endpoints. Use `PUT {trimmed}{HL_RANGE_SEP}{trimmed}:` to replace, or `CUT {trimmed}{HL_RANGE_SEP}{trimmed}` to delete."
        ));
    }
    let bare_range = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^([1-9]\d*)\s+(?:[1-9]\d*)\s*:?$").expect("static bare range regex") // safety: hardcoded compile-time regex literal
    });
    if bare_range.is_match(trimmed) {
        return Some(format!(
            "bare range hunk header {} is not valid. Hunk headers need a verb: use `PUT N{HL_RANGE_SEP}M:` or `CUT N{HL_RANGE_SEP}M`.",
            serde_json::to_string(trimmed).unwrap_or_default()
        ));
    }
    None
}

#[derive(Debug, Clone)]
struct PayloadRow {
    text: String,
    line_num: u64,
    bare: bool,
    minus: bool,
}

#[derive(Debug)]
struct Pending {
    target: BlockTarget,
    line_num: u64,
    payloads: Vec<PayloadRow>,
    had_colon: bool,
    deferred_blanks: Vec<PayloadRow>,
}

#[derive(Debug, Default)]
struct Executor {
    edits: Vec<Edit>,
    warnings: Vec<String>,
    edit_index: usize,
    pending: Option<Pending>,
    file_op: Option<FileOp>,
    terminated: bool,
    skippable_comments: Vec<(String, u64)>,
    /// Parse failures raised inside `flush_pending` (which cannot return a
    /// `Result`), surfaced on the next `feed`/`end` boundary.
    parse_failure: Option<String>,
}

impl Executor {
    fn discard_pending_skippable_comments(&mut self) {
        self.skippable_comments.clear();
    }

    fn consume_pending_skippable_comments(&mut self) -> Result<(), String> {
        if self.skippable_comments.is_empty() {
            return Ok(());
        }
        let comments = std::mem::take(&mut self.skippable_comments);
        for (text, line_num) in comments {
            self.handle_raw(&text, line_num)?;
        }
        Ok(())
    }

    fn feed(&mut self, token: Token) -> Result<(), String> {
        if self.terminated {
            return Ok(());
        }
        match token {
            Token::EnvelopeBegin { .. } => self.consume_pending_skippable_comments(),
            Token::EnvelopeEnd { .. } => {
                self.consume_pending_skippable_comments()?;
                self.terminated = true;
                Ok(())
            }
            Token::Abort { .. } => {
                self.terminated = true;
                Ok(())
            }
            Token::Header { .. } => {
                self.consume_pending_skippable_comments()?;
                self.flush_pending();
                Ok(())
            }
            Token::Blank { line_num } => {
                self.consume_pending_skippable_comments()?;
                self.handle_blank(line_num);
                Ok(())
            }
            Token::PayloadLiteral { text, line_num } => {
                self.consume_pending_skippable_comments()?;
                self.handle_literal_payload(&text, line_num)
            }
            Token::Raw { text, line_num } => {
                if self.pending.is_none() && is_skippable_comment_line(&text) {
                    self.skippable_comments.push((text, line_num));
                    return Ok(());
                }
                self.consume_pending_skippable_comments()?;
                self.handle_raw(&text, line_num)
            }
            Token::OpBlock {
                line_num,
                target,
                had_colon,
            } => {
                self.discard_pending_skippable_comments();
                match &target {
                    BlockTarget::Replace { range, register } => {
                        validate_range(
                            range,
                            line_num,
                            AbsoluteRangeOp::Replace,
                            register.as_deref(),
                        )?;
                    }
                    BlockTarget::Cut { range, register } => {
                        validate_range(range, line_num, AbsoluteRangeOp::Cut, register.as_deref())?;
                    }
                    _ => {}
                }
                if had_colon
                    && matches!(
                        target,
                        BlockTarget::Cut { .. } | BlockTarget::CutBlock { .. }
                    )
                    && !self
                        .warnings
                        .contains(&CUT_COLON_IGNORED_WARNING.to_string())
                {
                    self.warnings.push(CUT_COLON_IGNORED_WARNING.to_string());
                }
                if had_colon
                    && !matches!(target, BlockTarget::Rem | BlockTarget::Move { .. })
                    && target.register().is_some()
                {
                    return Err(format!("line {line_num}: {COLON_ON_REGISTER_PUT}"));
                }
                match target {
                    BlockTarget::Rem => {
                        self.flush_pending();
                        self.set_file_op(FileOp::Rem, line_num)
                    }
                    BlockTarget::Move { dest } => {
                        self.flush_pending();
                        self.set_file_op(FileOp::Move { dest }, line_num)
                    }
                    _ => {
                        self.flush_pending();
                        self.pending = Some(Pending {
                            target,
                            line_num,
                            payloads: Vec::new(),
                            had_colon,
                            deferred_blanks: Vec::new(),
                        });
                        Ok(())
                    }
                }
            }
        }
    }

    fn end(&mut self) -> Result<ParsedPatch, String> {
        self.consume_pending_skippable_comments()?;
        self.flush_pending();
        if let Some(error) = self.parse_failure.take() {
            return Err(error);
        }
        self.validate_file_op()?;
        self.normalize_overlapping_ranges()?;
        Ok((
            std::mem::take(&mut self.edits),
            self.file_op.clone(),
            std::mem::take(&mut self.warnings),
        ))
    }

    fn set_file_op(&mut self, file_op: FileOp, line_num: u64) -> Result<(), String> {
        if self.file_op.is_some() {
            return Err(format!(
                "line {line_num}: only one file-level op (`REM` or `MV`) per section. Merge them under one header."
            ));
        }
        if matches!(file_op, FileOp::Rem) && !self.edits.is_empty() {
            return Err(format!("line {line_num}: {REM_TAKES_NO_BODY}"));
        }
        self.file_op = Some(file_op);
        Ok(())
    }

    fn validate_file_op(&self) -> Result<(), String> {
        if matches!(self.file_op, Some(FileOp::Rem)) && !self.edits.is_empty() {
            return Err(
                "`REM` deletes the whole file and cannot be combined with line ops.".to_string(),
            );
        }
        Ok(())
    }

    fn normalize_overlapping_ranges(&mut self) -> Result<(), String> {
        #[derive(Default)]
        struct ConcreteHunk {
            line_num: u64,
            source_lines: std::collections::BTreeSet<u64>,
            clipboard_dependent: bool,
        }
        let mut hunks: std::collections::BTreeMap<u64, ConcreteHunk> =
            std::collections::BTreeMap::new();
        for edit in &self.edits {
            match edit {
                Edit::Cut { line_num, .. } => {
                    hunks.entry(*line_num).or_default().clipboard_dependent = true;
                }
                Edit::Paste {
                    at: PasteTarget::Span(range),
                    line_num,
                    ..
                } => {
                    let hunk = hunks.entry(*line_num).or_default();
                    hunk.clipboard_dependent = true;
                    for line in range.start.line..=range.end.line {
                        hunk.source_lines.insert(line);
                    }
                }
                Edit::Delete {
                    anchor, line_num, ..
                } => {
                    hunks
                        .entry(*line_num)
                        .or_default()
                        .source_lines
                        .insert(anchor.line);
                }
                _ => {}
            }
        }

        let mut owner_by_line: std::collections::BTreeMap<u64, u64> =
            std::collections::BTreeMap::new();
        let mut dropped: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        for hunk in hunks.values() {
            if hunk.source_lines.is_empty() {
                continue;
            }
            let mut overlaps: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
            let mut first_overlap: Option<u64> = None;
            for line in &hunk.source_lines {
                if let Some(owner) = owner_by_line.get(line) {
                    overlaps.insert(*owner);
                    if first_overlap.is_none() {
                        first_overlap = Some(*line);
                    }
                }
            }
            if overlaps.is_empty() {
                for line in &hunk.source_lines {
                    owner_by_line.insert(*line, hunk.line_num);
                }
                continue;
            }
            let previous = if overlaps.len() == 1 {
                overlaps.iter().next().copied()
            } else {
                None
            };
            let exact = previous.is_some_and(|prev| {
                let prev_hunk = &hunks[&prev];
                prev_hunk.source_lines.len() == hunk.source_lines.len()
                    && hunk
                        .source_lines
                        .iter()
                        .all(|line| prev_hunk.source_lines.contains(line))
                    && !prev_hunk.clipboard_dependent
            });
            if exact {
                // `exact` is only true when `previous` was Some, so this
                // binding always succeeds; if it ever did not, skipping the
                // coalesce fails closed instead of panicking.
                if let Some(previous) = previous {
                    dropped.insert(previous);
                    for line in &hunk.source_lines {
                        if owner_by_line.get(line) == Some(&previous) {
                            owner_by_line.remove(line);
                        }
                    }
                    for line in &hunk.source_lines {
                        owner_by_line.insert(*line, hunk.line_num);
                    }
                    if !self
                        .warnings
                        .contains(&REPLACE_PAIR_COALESCED_WARNING.to_string())
                    {
                        self.warnings
                            .push(REPLACE_PAIR_COALESCED_WARNING.to_string());
                    }
                }
                continue;
            }
            let previous_text = previous
                .map(|line| line.to_string())
                .unwrap_or_else(|| "an earlier line".to_string());
            // `first_overlap` is set in the same iteration that makes
            // `overlaps` non-empty (guarded by the `continue` above), so it
            // is always Some here; fall back to the hunk's own line number
            // rather than panicking on the impossible state.
            let first_overlap = first_overlap.unwrap_or(hunk.line_num);
            return Err(format!(
                "line {}: anchor line {} is already targeted by another hunk on line {}. Issue ONE hunk per range; payload is only the final desired content, never a before/after pair.",
                hunk.line_num, first_overlap, previous_text
            ));
        }
        if !dropped.is_empty() {
            self.edits
                .retain(|edit| !dropped.contains(&edit_line_num(edit)));
        }
        Ok(())
    }

    fn handle_literal_payload(&mut self, text: &str, line_num: u64) -> Result<(), String> {
        let pending = self.pending.as_mut().ok_or_else(|| {
            if self.file_op.is_some() {
                format!("line {line_num}: {MOVE_TAKES_NO_BODY}")
            } else {
                format!(
                    "line {line_num}: payload line has no preceding hunk header. Got {}.",
                    serde_json::to_string(&format!("{HL_PAYLOAD_REPLACE}{text}"))
                        .unwrap_or_default()
                )
            }
        })?;
        if let Some(message) = bodyless_target_message(&pending.target, pending.had_colon) {
            return Err(format!("line {line_num}: {message}"));
        }
        Self::commit_deferred_blanks(&mut self.warnings, pending);
        pending.payloads.push(PayloadRow {
            text: text.to_string(),
            line_num,
            bare: false,
            minus: false,
        });
        Ok(())
    }

    fn handle_raw(&mut self, text: &str, line_num: u64) -> Result<(), String> {
        if self.pending.is_none() && is_read_metadata_line(text) {
            if !self
                .warnings
                .contains(&READ_METADATA_IGNORED_WARNING.to_string())
            {
                self.warnings
                    .push(READ_METADATA_IGNORED_WARNING.to_string());
            }
            return Ok(());
        }
        if let Some(contamination) = detect_apply_patch_contamination(text) {
            return Err(format!("line {line_num}: {contamination}"));
        }
        if self.file_op.is_some() {
            return Err(format!("line {line_num}: {MOVE_TAKES_NO_BODY}"));
        }
        if let Some(pending) = self.pending.as_mut() {
            if text.trim().is_empty() {
                self.handle_blank(line_num);
                return Ok(());
            }
            if let Some(message) = bodyless_target_message(&pending.target, pending.had_colon) {
                return Err(format!("line {line_num}: {message}"));
            }
            let mut row = PayloadRow {
                text: text.to_string(),
                line_num,
                bare: true,
                minus: false,
            };
            if text.trim_start().starts_with('-') {
                row.minus = true;
            } else if !self
                .warnings
                .contains(&BARE_BODY_AUTO_PIPED_WARNING.to_string())
            {
                self.warnings.push(BARE_BODY_AUTO_PIPED_WARNING.to_string());
            }
            Self::commit_deferred_blanks(&mut self.warnings, pending);
            pending.payloads.push(row);
            return Ok(());
        }
        if text.trim().is_empty() {
            return Ok(());
        }
        if let Some(bare_range) = parse_top_level_bare_range_header(text) {
            validate_range(&bare_range, line_num, AbsoluteRangeOp::Replace, None)?;
            self.pending = Some(Pending {
                target: BlockTarget::Replace {
                    range: bare_range,
                    register: None,
                },
                line_num,
                payloads: Vec::new(),
                had_colon: true,
                deferred_blanks: Vec::new(),
            });
            if !self
                .warnings
                .contains(&BARE_RANGE_AUTO_PUT_WARNING.to_string())
            {
                self.warnings.push(BARE_RANGE_AUTO_PUT_WARNING.to_string());
            }
            return Ok(());
        }
        if let Some((line, snapshot_text)) = parse_top_level_snapshot_row(text) {
            let range = ParsedRange {
                start: Anchor { line },
                end: Anchor { line },
            };
            validate_range(&range, line_num, AbsoluteRangeOp::Replace, None)?;
            self.push_insert(
                Cursor::BeforeAnchor(Anchor { line }),
                snapshot_text,
                line_num,
                Some(InsertMode::Replacement),
                None,
            );
            self.push_delete_range(&range, line_num);
            if !self
                .warnings
                .contains(&SNAPSHOT_ROWS_AUTO_PUT_WARNING.to_string())
            {
                self.warnings
                    .push(SNAPSHOT_ROWS_AUTO_PUT_WARNING.to_string());
            }
            return Ok(());
        }
        Err(format!(
            "line {line_num}: payload line has no preceding hunk header. Use `PUT N{HL_RANGE_SEP}M:`, `CUT N{HL_RANGE_SEP}M`, or `PUT <N:`/`PUT >N:` above the body. Got {}.",
            serde_json::to_string(text).unwrap_or_default()
        ))
    }

    fn handle_blank(&mut self, line_num: u64) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if bodyless_target_message(&pending.target, pending.had_colon).is_some() {
            return;
        }
        if pending.payloads.is_empty() {
            return;
        }
        pending.deferred_blanks.push(PayloadRow {
            text: String::new(),
            line_num,
            bare: true,
            minus: false,
        });
    }

    fn commit_deferred_blanks(warnings: &mut Vec<String>, pending: &mut Pending) {
        if pending.deferred_blanks.is_empty() {
            return;
        }
        if !warnings.contains(&BARE_BODY_AUTO_PIPED_WARNING.to_string()) {
            warnings.push(BARE_BODY_AUTO_PIPED_WARNING.to_string());
        }
        pending
            .payloads
            .extend(std::mem::take(&mut pending.deferred_blanks));
    }

    fn resolve_minus_rows(&mut self, payloads: &mut Vec<PayloadRow>) -> Result<(), String> {
        let mut first_minus: Option<PayloadRow> = None;
        let mut all_bullet_shaped = true;
        let mut has_explicit = false;
        let mut has_explicit_bullet = false;
        for row in payloads.iter() {
            if row.minus {
                if first_minus.is_none() {
                    first_minus = Some(row.clone());
                }
                all_bullet_shaped &= MD_BULLET_ROW_RE.is_match(&row.text);
            } else if !row.bare {
                has_explicit = true;
                has_explicit_bullet |= MD_BULLET_ROW_RE.is_match(&row.text);
            }
        }
        let Some(first_minus) = first_minus else {
            return Ok(());
        };
        if all_bullet_shaped && (!has_explicit || has_explicit_bullet) {
            if !self
                .warnings
                .contains(&MINUS_BULLET_AUTO_PIPED_WARNING.to_string())
            {
                self.warnings
                    .push(MINUS_BULLET_AUTO_PIPED_WARNING.to_string());
            }
            return Ok(());
        }
        if has_explicit && !all_bullet_shaped {
            payloads.retain(|row| !row.minus);
            if !self
                .warnings
                .contains(&DIFF_OLD_ROWS_IGNORED_WARNING.to_string())
            {
                self.warnings
                    .push(DIFF_OLD_ROWS_IGNORED_WARNING.to_string());
            }
            return Ok(());
        }
        Err(format!(
            "line {}: {MINUS_ROW_REJECTED}",
            first_minus.line_num
        ))
    }

    fn strip_bare_prefixes_if_uniform(&mut self, payloads: &mut [PayloadRow]) {
        let mut saw_bare = false;
        let mut all_literal_values = true;
        for row in payloads.iter() {
            if !row.bare || row.text.trim().is_empty() {
                continue;
            }
            saw_bare = true;
            let stripped = strip_one_leading_hashline_prefix(&row.text);
            if stripped == row.text {
                return;
            }
            all_literal_values &= BARE_LITERAL_VALUE_RE.is_match(&stripped);
        }
        if !saw_bare || all_literal_values {
            return;
        }
        for row in payloads.iter_mut() {
            if row.bare && !row.text.trim().is_empty() {
                row.text = strip_one_leading_hashline_prefix(&row.text);
            }
        }
    }

    fn push_insert(
        &mut self,
        cursor: Cursor,
        text: String,
        line_num: u64,
        mode: Option<InsertMode>,
        block_start: Option<u64>,
    ) {
        self.edits.push(Edit::Insert {
            cursor,
            text,
            line_num,
            index: self.edit_index,
            mode,
            block_start,
        });
        self.edit_index += 1;
    }

    fn push_delete(&mut self, anchor: Anchor, line_num: u64) {
        self.edits.push(Edit::Delete {
            anchor,
            line_num,
            index: self.edit_index,
        });
        self.edit_index += 1;
    }

    fn push_delete_range(&mut self, range: &ParsedRange, line_num: u64) {
        for line in range.start.line..=range.end.line {
            self.push_delete(Anchor { line }, line_num);
        }
    }

    fn push_cut(&mut self, range: &ParsedRange, line_num: u64, register: Option<String>) {
        self.edits.push(Edit::Cut {
            range: ParsedRange {
                start: Anchor {
                    line: range.start.line,
                },
                end: Anchor {
                    line: range.end.line,
                },
            },
            register,
            line_num,
            index: self.edit_index,
        });
        self.edit_index += 1;
        self.push_delete_range(range, line_num);
    }

    fn push_paste(
        &mut self,
        at: PasteTarget,
        register: Option<String>,
        line_num: u64,
        block_start: Option<u64>,
    ) {
        self.edits.push(Edit::Paste {
            at,
            register,
            line_num,
            index: self.edit_index,
            block_start,
        });
        self.edit_index += 1;
    }

    fn push_block(
        &mut self,
        anchor: Anchor,
        payloads: Vec<String>,
        line_num: u64,
        mode: Option<BlockMode>,
        register: Option<String>,
    ) {
        self.edits.push(Edit::Block {
            anchor,
            payloads,
            mode,
            register,
            line_num,
            index: self.edit_index,
        });
        self.edit_index += 1;
    }

    fn emit_payload_rows(
        &mut self,
        cursor: Cursor,
        payloads: &[PayloadRow],
        line_num: u64,
        mode: Option<InsertMode>,
    ) {
        for payload in payloads {
            self.push_insert(cursor.clone(), payload.text.clone(), line_num, mode, None);
        }
    }

    fn flush_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mut payloads = pending.payloads;
        if let Err(error) = self.resolve_minus_rows(&mut payloads) {
            self.pending = Some(Pending {
                target: pending.target,
                line_num: pending.line_num,
                payloads,
                had_colon: pending.had_colon,
                deferred_blanks: pending.deferred_blanks,
            });
            self.parse_failure = Some(error);
            return;
        }
        self.strip_bare_prefixes_if_uniform(&mut payloads);
        match &pending.target {
            BlockTarget::Rem | BlockTarget::Move { .. } => return,
            BlockTarget::Cut { range, register } => {
                self.push_cut(range, pending.line_num, register.clone());
                return;
            }
            BlockTarget::CutBlock { anchor, register } => {
                self.push_block(
                    Anchor { line: anchor.line },
                    Vec::new(),
                    pending.line_num,
                    Some(BlockMode::Cut),
                    register.clone(),
                );
                return;
            }
            BlockTarget::Replace { range, register } => {
                if register.is_some() {
                    self.push_paste(
                        PasteTarget::Span(ParsedRange {
                            start: Anchor {
                                line: range.start.line,
                            },
                            end: Anchor {
                                line: range.end.line,
                            },
                        }),
                        register.clone(),
                        pending.line_num,
                        None,
                    );
                    return;
                }
                if payloads.is_empty() {
                    if !pending.had_colon {
                        self.parse_failure =
                            Some(format!("line {}: {COLONLESS_SPAN_PUT}", pending.line_num));
                        return;
                    }
                    self.push_delete_range(range, pending.line_num);
                    if !self
                        .warnings
                        .contains(&EMPTY_PUT_AUTO_CUT_WARNING.to_string())
                    {
                        self.warnings.push(EMPTY_PUT_AUTO_CUT_WARNING.to_string());
                    }
                    return;
                }
                let cursor = Cursor::BeforeAnchor(Anchor {
                    line: range.start.line,
                });
                self.emit_payload_rows(
                    cursor,
                    &payloads,
                    pending.line_num,
                    Some(InsertMode::Replacement),
                );
                self.push_delete_range(range, pending.line_num);
                return;
            }
            BlockTarget::Block { anchor, register } => {
                if register.is_some() {
                    self.push_block(
                        Anchor { line: anchor.line },
                        Vec::new(),
                        pending.line_num,
                        None,
                        register.clone(),
                    );
                    return;
                }
                if payloads.is_empty() {
                    if !pending.had_colon {
                        self.parse_failure =
                            Some(format!("line {}: {COLONLESS_SPAN_PUT}", pending.line_num));
                        return;
                    }
                    self.push_block(
                        Anchor { line: anchor.line },
                        Vec::new(),
                        pending.line_num,
                        None,
                        None,
                    );
                    if !self
                        .warnings
                        .contains(&EMPTY_PUT_AUTO_CUT_WARNING.to_string())
                    {
                        self.warnings.push(EMPTY_PUT_AUTO_CUT_WARNING.to_string());
                    }
                    return;
                }
                self.push_block(
                    Anchor { line: anchor.line },
                    payloads.iter().map(|row| row.text.clone()).collect(),
                    pending.line_num,
                    None,
                    None,
                );
                return;
            }
            BlockTarget::InsertAfterBlock { anchor, register } => {
                if register.is_some() || (!pending.had_colon && payloads.is_empty()) {
                    self.push_block(
                        Anchor { line: anchor.line },
                        Vec::new(),
                        pending.line_num,
                        Some(BlockMode::PasteAfter),
                        register.clone(),
                    );
                    return;
                }
                if payloads.is_empty() {
                    self.parse_failure = Some(format!("line {}: {EMPTY_INSERT}", pending.line_num));
                    return;
                }
                self.push_block(
                    Anchor { line: anchor.line },
                    payloads.iter().map(|row| row.text.clone()).collect(),
                    pending.line_num,
                    Some(BlockMode::InsertAfter),
                    None,
                );
                return;
            }
            _ => {}
        }
        let cursor = match &pending.target {
            BlockTarget::InsertBefore { anchor, .. } => {
                Cursor::BeforeAnchor(Anchor { line: anchor.line })
            }
            BlockTarget::InsertAfter { anchor, .. } => {
                Cursor::AfterAnchor(Anchor { line: anchor.line })
            }
            BlockTarget::Bof { .. } => Cursor::Bof,
            BlockTarget::Eof { .. } => Cursor::Eof,
            // Every non-gap target returned from the match above, so this
            // arm cannot be reached; fail closed by recording a parse
            // failure instead of panicking.
            _ => {
                self.parse_failure = Some(format!(
                    "line {}: unsupported target for gap insertion",
                    pending.line_num
                ));
                return;
            }
        };
        let register = pending.target.register().map(ToString::to_string);
        if register.is_some() || (!pending.had_colon && payloads.is_empty()) {
            self.push_paste(PasteTarget::Gap(cursor), register, pending.line_num, None);
            return;
        }
        if payloads.is_empty() {
            self.parse_failure = Some(format!("line {}: {EMPTY_INSERT}", pending.line_num));
            return;
        }
        self.emit_payload_rows(cursor, &payloads, pending.line_num, None);
    }
}

fn edit_line_num(edit: &Edit) -> u64 {
    match edit {
        Edit::Insert { line_num, .. }
        | Edit::Delete { line_num, .. }
        | Edit::Cut { line_num, .. }
        | Edit::Paste { line_num, .. }
        | Edit::Block { line_num, .. } => *line_num,
    }
}

const SNAPSHOT_ROWS_AUTO_PUT_WARNING: &str = hl_const!(
    "Recovered top-level `N:TEXT` snapshot row(s) as single-line `PUT N",
    ".=",
    "N:` replacements. Use explicit `PUT` headers for reliable edits."
);

const BARE_RANGE_AUTO_PUT_WARNING: &str = hl_const!(
    "Recovered a bare `N",
    ".=",
    "M:` header as `PUT N",
    ".=",
    "M:`. Prefix replacement ranges with `PUT`."
);

/// A parsed hashline patch: the concrete edits, an optional file-level op
/// (REM/MV), and the parse warnings emitted while tokenizing.
pub(crate) type ParsedPatch = (Vec<Edit>, Option<FileOp>, Vec<String>);

/// `parsePatch`: tokenize + execute a section diff body. The executor
/// defers parse failures raised inside `flush_pending` into
/// `parse_failure` so they surface here with the exact line-scoped text.
pub(crate) fn parse_patch(diff: &str) -> Result<ParsedPatch, String> {
    let mut executor = Executor::default();
    for (index, line) in split_hashline_lines(diff).iter().enumerate() {
        executor.feed(classify_line(line, index as u64 + 1))?;
        if let Some(error) = executor.parse_failure.take() {
            return Err(error);
        }
    }
    executor.end()
}

// ═══════════════════════════════════════════════════════════════════════════
// clipboard — pinned `packages/hashline/src/clipboard.ts`
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Clipboard {
    /// Anonymous register: lines captured by the latest unlabeled `CUT`.
    pub(crate) lines: Option<Vec<String>>,
    /// Named registers captured by `CUT … @name`.
    pub(crate) named: std::collections::BTreeMap<String, Vec<String>>,
    /// Headers of unlabeled `CUT`s seen since the last unlabeled paste.
    pub(crate) pending_anon_cuts: Vec<String>,
}

fn describe_cut_edit(edit: &Edit) -> String {
    if let Edit::Cut {
        range, register, ..
    } = edit
    {
        let span = if range.start.line == range.end.line {
            format!("{}", range.start.line)
        } else {
            format!("{}{HL_RANGE_SEP}{}", range.start.line, range.end.line)
        };
        let reg = register
            .as_ref()
            .map(|reg| format!(" @{reg}"))
            .unwrap_or_default();
        return format!("{HL_CUT_KEYWORD} {span}{reg}");
    }
    String::new()
}

fn has_clipboard_edit(edits: &[Edit]) -> bool {
    edits.iter().any(|edit| match edit {
        Edit::Cut { .. } | Edit::Paste { .. } => true,
        Edit::Block { mode, register, .. } => {
            matches!(mode, Some(BlockMode::Cut) | Some(BlockMode::PasteAfter)) || register.is_some()
        }
        _ => false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnEmptyPaste {
    Throw,
    Drop,
}

fn read_register(
    register: Option<&str>,
    clipboard: &mut Clipboard,
    line_num: u64,
    on_empty_paste: OnEmptyPaste,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<String>>, String> {
    if let Some(register) = register {
        if let Some(lines) = clipboard.named.get(register) {
            return Ok(Some(lines.clone()));
        }
        if on_empty_paste == OnEmptyPaste::Drop {
            return Ok(None);
        }
        let known: Vec<String> = clipboard.named.keys().cloned().collect();
        warnings.push(format!(
            "line {line_num}: {}",
            empty_register_paste_warning(register, &known)
        ));
        return Ok(Some(Vec::new()));
    }
    let pending = clipboard.pending_anon_cuts.clone();
    if pending.len() > 1 {
        if on_empty_paste == OnEmptyPaste::Drop {
            return Ok(None);
        }
        return Err(format!(
            "line {line_num}: {}",
            ambiguous_anonymous_paste_message(&pending)
        ));
    }
    let Some(lines) = clipboard.lines.clone() else {
        if on_empty_paste == OnEmptyPaste::Drop {
            return Ok(None);
        }
        return Err(format!("line {line_num}: {EMPTY_PASTE}"));
    };
    clipboard.pending_anon_cuts.clear();
    Ok(Some(lines))
}

fn write_register(
    edit: &Edit,
    file_lines: &[String],
    clipboard: &mut Clipboard,
) -> Result<(), String> {
    if let Edit::Cut {
        range,
        register,
        line_num,
        ..
    } = edit
    {
        if range.start.line < 1 || range.end.line > file_lines.len() as u64 {
            return Err(format!(
                "line {line_num}: `{}` is out of range (file has {} lines).",
                describe_cut_edit(edit),
                file_lines.len()
            ));
        }
        let captured: Vec<String> =
            file_lines[(range.start.line - 1) as usize..range.end.line as usize].to_vec();
        if let Some(register) = register {
            clipboard.named.insert(register.clone(), captured);
        } else {
            clipboard.lines = Some(captured);
            clipboard.pending_anon_cuts.push(describe_cut_edit(edit));
        }
    }
    Ok(())
}

/// `resolveClipboardEdits`: cuts fill registers and emit nothing; pastes
/// expand to inserts (+ deletes for span targets).
fn resolve_clipboard_edits(
    edits: &[Edit],
    file_lines: &[String],
    clipboard: &mut Clipboard,
    on_empty_paste: OnEmptyPaste,
    warnings: &mut Vec<String>,
) -> Result<Vec<Edit>, String> {
    if !has_clipboard_edit(edits) {
        return Ok(edits.to_vec());
    }
    let mut resolved: Vec<Edit> = Vec::new();
    let mut synth_index = 0usize;
    for edit in edits {
        match edit {
            Edit::Cut { .. } => {
                write_register(edit, file_lines, clipboard)?;
            }
            Edit::Paste {
                at,
                register,
                line_num,
                block_start,
                ..
            } => {
                let lines = read_register(
                    register.as_deref(),
                    clipboard,
                    *line_num,
                    on_empty_paste,
                    warnings,
                )?;
                if let Some(lines) = lines {
                    match at {
                        PasteTarget::Gap(cursor) => {
                            for text in lines {
                                resolved.push(Edit::Insert {
                                    cursor: cursor.clone(),
                                    text,
                                    line_num: *line_num,
                                    index: synth_index,
                                    mode: None,
                                    block_start: *block_start,
                                });
                                synth_index += 1;
                            }
                        }
                        PasteTarget::Span(range) => {
                            if range.start.line < 1 || range.end.line > file_lines.len() as u64 {
                                let reg = register
                                    .as_ref()
                                    .map(|reg| format!(" @{reg}"))
                                    .unwrap_or_default();
                                return Err(format!(
                                    "line {line_num}: `{HL_PUT_KEYWORD} {}{HL_RANGE_SEP}{}{reg}` is out of range (file has {} lines).",
                                    range.start.line,
                                    range.end.line,
                                    file_lines.len()
                                ));
                            }
                            let cursor = Cursor::BeforeAnchor(Anchor {
                                line: range.start.line,
                            });
                            for text in lines {
                                resolved.push(Edit::Insert {
                                    cursor: cursor.clone(),
                                    text,
                                    line_num: *line_num,
                                    index: synth_index,
                                    mode: Some(InsertMode::Replacement),
                                    block_start: None,
                                });
                                synth_index += 1;
                            }
                            for line in range.start.line..=range.end.line {
                                resolved.push(Edit::Delete {
                                    anchor: Anchor { line },
                                    line_num: *line_num,
                                    index: synth_index,
                                });
                                synth_index += 1;
                            }
                        }
                    }
                }
            }
            other => resolved.push(other.clone()),
        }
    }
    Ok(resolved)
}

// ═══════════════════════════════════════════════════════════════════════════
// apply — pinned `packages/hashline/src/apply.ts` (core; no heuristic repairs)
// ═══════════════════════════════════════════════════════════════════════════

fn get_cursor_anchors(cursor: &Cursor) -> Vec<Anchor> {
    match cursor {
        Cursor::BeforeAnchor(anchor) | Cursor::AfterAnchor(anchor) => vec![*anchor],
        _ => Vec::new(),
    }
}

fn get_edit_anchors(edit: &Edit) -> Vec<Anchor> {
    match edit {
        Edit::Delete { anchor, .. } => vec![*anchor],
        Edit::Insert { cursor, .. } => get_cursor_anchors(cursor),
        _ => Vec::new(),
    }
}

/// `trailingPhantomLine`: `split("\n")` on a newline-terminated file yields
/// a trailing "" sentinel; deleting it only strips the file's final newline.
fn trailing_phantom_line(file_lines: &[String]) -> u64 {
    if file_lines.len() > 1 && file_lines.last().is_some_and(|line| line.is_empty()) {
        file_lines.len() as u64
    } else {
        0
    }
}

fn drop_trailing_phantom_deletes(edits: Vec<Edit>, file_lines: &[String]) -> Vec<Edit> {
    let phantom_line = trailing_phantom_line(file_lines);
    if phantom_line == 0 {
        return edits;
    }
    edits
        .into_iter()
        .filter(|edit| !matches!(edit, Edit::Delete { anchor, .. } if anchor.line == phantom_line))
        .collect()
}

/// `validateLineBounds`: verify every anchored edit points at an existing line.
fn validate_line_bounds(edits: &[Edit], file_lines: &[String]) -> Result<(), String> {
    for edit in edits {
        for anchor in get_edit_anchors(edit) {
            if anchor.line < 1 || anchor.line > file_lines.len() as u64 {
                return Err(line_out_of_bounds(anchor.line, file_lines.len()));
            }
        }
    }
    Ok(())
}

fn insert_at_start(file_lines: &mut Vec<String>, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    if file_lines.len() == 1 && file_lines[0].is_empty() {
        file_lines.splice(0..1, lines.iter().cloned());
        return;
    }
    file_lines.splice(0..0, lines.iter().cloned());
}

fn insert_at_end(file_lines: &mut Vec<String>, lines: &[String]) -> Option<u64> {
    if lines.is_empty() {
        return None;
    }
    if file_lines.len() == 1 && file_lines[0].is_empty() {
        file_lines.splice(0..1, lines.iter().cloned());
        return Some(1);
    }
    let has_trailing_newline = file_lines.last().is_some_and(|line| line.is_empty());
    let insert_index = if has_trailing_newline {
        file_lines.len() - 1
    } else {
        file_lines.len()
    };
    file_lines.splice(insert_index..insert_index, lines.iter().cloned());
    Some(insert_index as u64 + 1)
}

fn is_replacement_insert(edit: &Edit) -> bool {
    matches!(
        edit,
        Edit::Insert {
            mode: Some(InsertMode::Replacement),
            ..
        }
    )
}

/// `applyEdits` core: partition bof/eof/anchor buckets, apply bottom-up so
/// earlier indices stay valid.
pub(crate) fn apply_edits(
    text: &str,
    edits: &[Edit],
    clipboard: &mut Clipboard,
) -> Result<ApplyResult, String> {
    if edits.is_empty() {
        return Ok(ApplyResult::noop(text.to_string()));
    }

    let file_lines: Vec<String> = text.split('\n').map(ToString::to_string).collect();

    let mut clipboard_warnings: Vec<String> = Vec::new();
    let concrete = resolve_clipboard_edits(
        edits,
        &file_lines,
        clipboard,
        OnEmptyPaste::Throw,
        &mut clipboard_warnings,
    )?;

    for edit in &concrete {
        if matches!(edit, Edit::Block { .. }) {
            return Err(
                "internal error: unresolved block edit reached the applier (resolveBlockEdits was not run)."
                    .to_string(),
            );
        }
    }

    let mut first_changed_line: Option<u64> = None;
    // Pinned `trackFirstChanged`: the FIRST (lowest) line touched by any
    // edit, across all buckets — buckets apply bottom-up, so keeping only
    // the first-set value would report the highest changed line instead.
    let mut track_first_changed = |line: u64| {
        if first_changed_line.is_none_or(|first| line < first) {
            first_changed_line = Some(line);
        }
    };

    let target_edits = drop_trailing_phantom_deletes(concrete, &file_lines);
    validate_line_bounds(&target_edits, &file_lines)?;

    let mut file_lines = file_lines;
    let mut bof_lines: Vec<String> = Vec::new();
    let mut eof_lines: Vec<String> = Vec::new();
    let mut anchor_edits: Vec<(Edit, usize)> = Vec::new();
    for (idx, edit) in target_edits.iter().enumerate() {
        match edit {
            Edit::Insert {
                cursor: Cursor::Bof,
                text,
                ..
            } => bof_lines.push(text.clone()),
            Edit::Insert {
                cursor: Cursor::Eof,
                text,
                ..
            } => eof_lines.push(text.clone()),
            other => anchor_edits.push((other.clone(), idx)),
        }
    }

    let mut by_line: std::collections::BTreeMap<u64, Vec<(Edit, usize)>> =
        std::collections::BTreeMap::new();
    for entry in anchor_edits {
        let line = match &entry.0 {
            Edit::Delete { anchor, .. } => anchor.line,
            Edit::Insert {
                cursor: Cursor::BeforeAnchor(anchor) | Cursor::AfterAnchor(anchor),
                ..
            } => anchor.line,
            _ => 0,
        };
        by_line.entry(line).or_default().push(entry);
    }

    for (line, mut bucket) in by_line.into_iter().rev() {
        bucket.sort_by_key(|(_, idx)| *idx);
        let idx = (line - 1) as usize;
        let current_line = file_lines.get(idx).cloned().unwrap_or_default();
        let mut before_insert_lines: Vec<String> = Vec::new();
        let mut after_insert_lines: Vec<String> = Vec::new();
        let mut replacement_lines: Vec<String> = Vec::new();
        let mut delete_line = false;
        for (edit, _) in &bucket {
            if is_replacement_insert(edit) {
                if let Edit::Insert { text, .. } = edit {
                    replacement_lines.push(text.clone());
                }
            } else if let Edit::Insert {
                cursor: Cursor::AfterAnchor(_),
                text,
                ..
            } = edit
            {
                after_insert_lines.push(text.clone());
            } else if let Edit::Insert { text, .. } = edit {
                before_insert_lines.push(text.clone());
            } else if matches!(edit, Edit::Delete { .. }) {
                delete_line = true;
            }
        }
        if before_insert_lines.is_empty()
            && replacement_lines.is_empty()
            && after_insert_lines.is_empty()
            && !delete_line
        {
            continue;
        }
        let mut replacement: Vec<String> = Vec::with_capacity(
            before_insert_lines.len() + replacement_lines.len() + after_insert_lines.len() + 1,
        );
        replacement.extend(before_insert_lines.iter().cloned());
        replacement.extend(replacement_lines.iter().cloned());
        if !delete_line {
            replacement.push(current_line);
        }
        replacement.extend(after_insert_lines.iter().cloned());
        file_lines.splice(idx..idx + 1, replacement);
        track_first_changed(line);
    }

    if !bof_lines.is_empty() {
        insert_at_start(&mut file_lines, &bof_lines);
        track_first_changed(1);
    }
    if let Some(changed) = insert_at_end(&mut file_lines, &eof_lines) {
        track_first_changed(changed);
    }

    Ok(ApplyResult {
        text: file_lines.join("\n"),
        first_changed_line,
        warnings: clipboard_warnings,
        block_resolutions: Vec::new(),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// block — pinned `packages/hashline/src/block.ts` with a lexical
// brace-matching resolver standing in for the tree-sitter native resolver
// ═══════════════════════════════════════════════════════════════════════════

static STRUCTURAL_CLOSER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*[)\]}]+[;,]?\s*$").expect("static closer regex") // safety: hardcoded compile-time regex literal
});

/// A line that is nothing but closing delimiters: `}`, `)`, `];`, `})`, `},`.
fn is_structural_closer_line(text: &str) -> bool {
    STRUCTURAL_CLOSER_RE.is_match(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannerMode {
    Code,
    Single,
    Double,
    Template,
    BlockComment,
}

/// Lexical block resolver: finds the first `{` outside strings/comments on
/// the anchor line, then matches it to its closing brace. Mirrors the
/// pinned lexical bracket fallback's string/comment handling. Returns the
/// 1-indexed inclusive span, or `None` when line N does not open a block.
fn lexical_block_resolver(text: &str, line: u64) -> Option<(u64, u64)> {
    let lines: Vec<&str> = text.split('\n').collect();
    if line < 1 || line as usize > lines.len() {
        return None;
    }
    let mut mode: ScannerMode = ScannerMode::Code;
    let mut escaped = false;
    for (line_index, line_text) in lines.iter().enumerate() {
        let line_number = line_index as u64 + 1;
        let mut index = 0usize;
        while index < line_text.len() {
            // `index` advances only by char-boundary widths and stays below
            // `line_text.len()`, so the slice is always non-empty; stop
            // scanning this line instead of panicking on the impossible
            // case.
            let Some(ch) = line_text[index..].chars().next() else {
                break;
            };
            let next = line_text[index + ch.len_utf8()..].chars().next();
            match mode {
                ScannerMode::BlockComment => {
                    if ch == '*' && next == Some('/') {
                        mode = ScannerMode::Code;
                        index += 2;
                        continue;
                    }
                    index += ch.len_utf8();
                    continue;
                }
                ScannerMode::Single | ScannerMode::Double | ScannerMode::Template => {
                    if escaped {
                        escaped = false;
                        index += ch.len_utf8();
                        continue;
                    }
                    if ch == '\\' {
                        escaped = true;
                        index += ch.len_utf8();
                        continue;
                    }
                    let closing = match mode {
                        ScannerMode::Single => '\'',
                        ScannerMode::Double => '"',
                        _ => '`',
                    };
                    if ch == closing {
                        mode = ScannerMode::Code;
                    }
                    index += ch.len_utf8();
                    continue;
                }
                ScannerMode::Code => {}
            }
            if ch == '/' && next == Some('/') {
                break;
            }
            if ch == '/' && next == Some('*') {
                mode = ScannerMode::BlockComment;
                index += 2;
                continue;
            }
            if ch == '\'' {
                mode = ScannerMode::Single;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '"' {
                mode = ScannerMode::Double;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '`' {
                mode = ScannerMode::Template;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '{' && line_number == line {
                return match_braces(&lines, line_number, index, mode, escaped);
            }
            // Braces on earlier lines are just depth noise for the
            // anchor-line scan; only anchor-line braces matter.
            index += ch.len_utf8();
        }
        if matches!(mode, ScannerMode::Single | ScannerMode::Double) {
            mode = ScannerMode::Code;
            escaped = false;
        }
        if line_number >= line {
            break;
        }
    }
    None
}

/// Match the brace opened at `open_line`/`open_col` to its closer; returns
/// the 1-indexed inclusive line span.
fn match_braces(
    lines: &[&str],
    open_line: u64,
    open_col: usize,
    mode: ScannerMode,
    escaped: bool,
) -> Option<(u64, u64)> {
    let mut depth = 0i64;
    let mut mode = mode;
    let mut escaped = escaped;
    for (line_index, line_text) in lines.iter().enumerate() {
        let line_number = line_index as u64 + 1;
        if line_number < open_line {
            continue;
        }
        let start_col = if line_number == open_line {
            open_col
        } else {
            0
        };
        let mut index = start_col;
        while index < line_text.len() {
            // `index` advances only by char-boundary widths and stays below
            // `line_text.len()`, so the slice is always non-empty; stop
            // scanning this line instead of panicking on the impossible
            // case.
            let Some(ch) = line_text[index..].chars().next() else {
                break;
            };
            let next = line_text[index + ch.len_utf8()..].chars().next();
            match mode {
                ScannerMode::BlockComment => {
                    if ch == '*' && next == Some('/') {
                        mode = ScannerMode::Code;
                        index += 2;
                        continue;
                    }
                    index += ch.len_utf8();
                    continue;
                }
                ScannerMode::Single | ScannerMode::Double | ScannerMode::Template => {
                    if escaped {
                        escaped = false;
                        index += ch.len_utf8();
                        continue;
                    }
                    if ch == '\\' {
                        escaped = true;
                        index += ch.len_utf8();
                        continue;
                    }
                    let closing = match mode {
                        ScannerMode::Single => '\'',
                        ScannerMode::Double => '"',
                        _ => '`',
                    };
                    if ch == closing {
                        mode = ScannerMode::Code;
                    }
                    index += ch.len_utf8();
                    continue;
                }
                ScannerMode::Code => {}
            }
            if ch == '/' && next == Some('/') {
                break;
            }
            if ch == '/' && next == Some('*') {
                mode = ScannerMode::BlockComment;
                index += 2;
                continue;
            }
            if ch == '\'' {
                mode = ScannerMode::Single;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '"' {
                mode = ScannerMode::Double;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '`' {
                mode = ScannerMode::Template;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    return Some((open_line, line_number));
                }
            }
            index += ch.len_utf8();
        }
        if matches!(mode, ScannerMode::Single | ScannerMode::Double) {
            mode = ScannerMode::Code;
            escaped = false;
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnUnresolved {
    Throw,
    Drop,
}

/// `resolveBlockEdits`: expand deferred block edits into concrete edits.
pub(crate) fn resolve_block_edits(
    edits: &[Edit],
    text: &str,
    on_unresolved: OnUnresolved,
    warnings: &mut Vec<String>,
    resolutions: &mut Vec<BlockResolution>,
) -> Result<Vec<Edit>, String> {
    if !edits.iter().any(|edit| matches!(edit, Edit::Block { .. })) {
        return Ok(edits.to_vec());
    }
    let mut resolved: Vec<Edit> = Vec::new();
    let mut synth_index = 0usize;
    for edit in edits {
        let Edit::Block {
            anchor,
            payloads,
            mode,
            register,
            line_num,
            ..
        } = edit
        else {
            resolved.push(edit.clone());
            continue;
        };
        let op = match mode {
            Some(BlockMode::InsertAfter) => BlockOp::InsertAfter,
            Some(BlockMode::Cut) => BlockOp::Cut,
            Some(BlockMode::PasteAfter) => BlockOp::PasteAfter,
            None => BlockOp::Replace,
        };
        let span = lexical_block_resolver(text, anchor.line);
        let Some((start, end)) = span else {
            if op == BlockOp::InsertAfter || op == BlockOp::PasteAfter {
                let anchor_text = text.split('\n').nth((anchor.line - 1) as usize);
                let is_closer = anchor_text.is_some_and(is_structural_closer_line);
                if op == BlockOp::PasteAfter {
                    warnings.push(if is_closer {
                        paste_after_block_closer_lowered_warning(anchor.line)
                    } else {
                        paste_after_block_unresolved_lowered_warning(anchor.line)
                    });
                    resolved.push(Edit::Paste {
                        at: PasteTarget::Gap(Cursor::AfterAnchor(Anchor { line: anchor.line })),
                        register: register.clone(),
                        line_num: *line_num,
                        index: synth_index,
                        block_start: None,
                    });
                    synth_index += 1;
                    continue;
                }
                warnings.push(if is_closer {
                    insert_after_block_closer_lowered_warning(anchor.line)
                } else {
                    insert_after_block_unresolved_lowered_warning(anchor.line)
                });
                for payload in payloads {
                    resolved.push(Edit::Insert {
                        cursor: Cursor::AfterAnchor(Anchor { line: anchor.line }),
                        text: payload.clone(),
                        line_num: *line_num,
                        index: synth_index,
                        mode: None,
                        block_start: None,
                    });
                    synth_index += 1;
                }
                continue;
            }
            if on_unresolved == OnUnresolved::Drop {
                continue;
            }
            let lines: Vec<String> = text.split('\n').map(ToString::to_string).collect();
            let suggestions = BlockDiagnosticSuggestions::default();
            let op = match op {
                BlockOp::Replace => AbsoluteRangeOp::Replace,
                BlockOp::Cut => AbsoluteRangeOp::Cut,
                // `InsertAfter` and `PasteAfter` returned from the branch
                // above, so this arm cannot be reached; fail closed with the
                // unresolved-block error path instead of panicking.
                BlockOp::InsertAfter | BlockOp::PasteAfter => {
                    return Err(format!(
                        "line {line_num}: unsupported insert/paste-after for an unresolved block"
                    ));
                }
            };
            return Err(format!(
                "line {line_num}: {}",
                block_unresolved_message(
                    anchor.line,
                    op,
                    Some(&lines),
                    &suggestions,
                    register.as_deref()
                )
            ));
        };
        if start == end {
            if on_unresolved == OnUnresolved::Drop {
                continue;
            }
            return Err(format!(
                "line {line_num}: {}",
                block_single_line_message(anchor.line, op, None)
            ));
        }
        resolutions.push(BlockResolution {
            anchor_line: anchor.line,
            start,
            end,
            op,
        });
        match op {
            BlockOp::PasteAfter => {
                resolved.push(Edit::Paste {
                    at: PasteTarget::Gap(Cursor::AfterAnchor(Anchor { line: end })),
                    register: register.clone(),
                    line_num: *line_num,
                    index: synth_index,
                    block_start: Some(start),
                });
                synth_index += 1;
            }
            BlockOp::Cut => {
                resolved.push(Edit::Cut {
                    range: ParsedRange {
                        start: Anchor { line: start },
                        end: Anchor { line: end },
                    },
                    register: register.clone(),
                    line_num: *line_num,
                    index: synth_index,
                });
                synth_index += 1;
                for line in start..=end {
                    resolved.push(Edit::Delete {
                        anchor: Anchor { line },
                        line_num: *line_num,
                        index: synth_index,
                    });
                    synth_index += 1;
                }
            }
            BlockOp::InsertAfter => {
                for payload in payloads {
                    resolved.push(Edit::Insert {
                        cursor: Cursor::AfterAnchor(Anchor { line: end }),
                        text: payload.clone(),
                        line_num: *line_num,
                        index: synth_index,
                        mode: None,
                        block_start: Some(start),
                    });
                    synth_index += 1;
                }
            }
            BlockOp::Replace => {
                if register.is_some() {
                    resolved.push(Edit::Paste {
                        at: PasteTarget::Span(ParsedRange {
                            start: Anchor { line: start },
                            end: Anchor { line: end },
                        }),
                        register: register.clone(),
                        line_num: *line_num,
                        index: synth_index,
                        block_start: None,
                    });
                    synth_index += 1;
                } else {
                    for payload in payloads {
                        resolved.push(Edit::Insert {
                            cursor: Cursor::BeforeAnchor(Anchor { line: start }),
                            text: payload.clone(),
                            line_num: *line_num,
                            index: synth_index,
                            mode: Some(InsertMode::Replacement),
                            block_start: None,
                        });
                        synth_index += 1;
                    }
                    for line in start..=end {
                        resolved.push(Edit::Delete {
                            anchor: Anchor { line },
                            line_num: *line_num,
                            index: synth_index,
                        });
                        synth_index += 1;
                    }
                }
            }
        }
    }
    Ok(resolved)
}

// ═══════════════════════════════════════════════════════════════════════════
// input split — pinned `packages/hashline/src/input.ts`
// ═══════════════════════════════════════════════════════════════════════════

static APPLY_PATCH_PATH_NOISE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(
    || {
        regex::Regex::new(r"^\*{0,3}\s*(?:(?:update|add|delete|move)[^A-Za-z0-9]*(?:file|to)?[^A-Za-z0-9]*:)?\s*\*{0,3}\s*")
        .expect("static noise regex") // safety: hardcoded compile-time regex literal
    },
);

fn strip_apply_patch_path_noise(path_text: &str) -> String {
    APPLY_PATCH_PATH_NOISE_RE
        .replace(path_text, "")
        .into_owned()
}

fn unquote_hashline_path(path_text: &str) -> String {
    if path_text.len() < 2 {
        return path_text.to_string();
    }
    let bytes = path_text.as_bytes();
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    if (first == b'"' || first == b'\'') && first == last {
        path_text[1..path_text.len() - 1].to_string()
    } else {
        path_text.to_string()
    }
}

/// `normalizeHashlinePath` (cwd-relative form is a no-op here: engine paths
/// are workspace-relative by construction).
fn normalize_hashline_path(raw_path: &str) -> String {
    strip_apply_patch_path_noise(&unquote_hashline_path(raw_path.trim()))
}

fn try_parse_recovery_header(line: &str) -> Option<(String, Option<String>)> {
    if !line.starts_with(HL_FILE_PREFIX) || !line.ends_with(HL_FILE_SUFFIX) {
        return None;
    }
    let body = strip_apply_patch_path_noise(
        line[HL_FILE_PREFIX.len()..line.len() - HL_FILE_SUFFIX.len()].trim(),
    );
    if body.is_empty() {
        return None;
    }
    let trailing = std::sync::LazyLock::new(|| {
        // `HL_FILE_HASH_LENGTH` is a const, so the formatted pattern is a
        // fixed literal that always compiles.
        regex::Regex::new(&format!(r"#([0-9A-Fa-f]{{{HL_FILE_HASH_LENGTH}}})\s*$"))
            .expect("static trailing tag regex") // safety: pattern built from const HL_FILE_HASH_LENGTH
    });
    // The pattern's only capture group is mandatory, so `get(1)` is Some
    // whenever the overall match succeeded; the `None` arm below (treated
    // as "no tag") fails closed rather than panicking.
    let (path_text, file_hash) = match trailing
        .captures(&body)
        .and_then(|captures| captures.get(1))
    {
        Some(hash) => (
            body[..hash.start()].to_string(),
            Some(hash.as_str().to_ascii_uppercase()),
        ),
        None => (body.trim_end().to_string(), None),
    };
    if path_text.contains('#') {
        return None;
    }
    let path = normalize_hashline_path(&path_text);
    if path.is_empty() {
        return None;
    }
    Some((path, file_hash))
}

/// `parseHashlineHeaderLine`: strict parse with apply_patch-noise recovery.
fn parse_hashline_header_line(line: &str) -> Result<Option<(String, Option<String>)>, String> {
    let trimmed = line.trim_end();
    if !trimmed.starts_with(HL_FILE_PREFIX) {
        return Ok(None);
    }
    if let Some((path, file_hash)) = try_parse_header(trimmed) {
        let parsed_path = normalize_hashline_path(&path);
        if parsed_path.is_empty() {
            return Err(format!(
                "Input header \"{HL_FILE_PREFIX}{HL_FILE_SUFFIX}\" is empty; provide a file path."
            ));
        }
        return Ok(Some((parsed_path, file_hash)));
    }
    if let Some(recovered) = try_parse_recovery_header(trimmed) {
        return Ok(Some(recovered));
    }
    Err(format!(
        "Input header must be {HL_FILE_PREFIX}PATH{HL_FILE_SUFFIX} or {HL_FILE_PREFIX}PATH{HL_FILE_HASH_SEP}TAG{HL_FILE_SUFFIX} with a {HL_FILE_HASH_LENGTH}-hex content-hash tag; got {}.",
        serde_json::to_string(trimmed).unwrap_or_default()
    ))
}

fn strip_leading_blank_lines(input: &str) -> String {
    let stripped = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    let mut lines: Vec<&str> = stripped.split('\n').collect();
    while !lines.is_empty() {
        let head = lines[0].strip_suffix('\r').unwrap_or(lines[0]);
        if head.trim().is_empty() || head.trim() == BEGIN_PATCH_MARKER {
            lines.remove(0);
            continue;
        }
        break;
    }
    lines.join("\n")
}

fn flush_raw_section(
    sections: &mut Vec<(String, Option<String>, String)>,
    current: &mut Option<(String, Option<String>)>,
    current_lines: &mut Vec<String>,
) {
    if let Some((path, file_hash)) = current.take() {
        let has_ops = current_lines.iter().any(|line| !line.trim().is_empty());
        if has_ops {
            sections.push((path, file_hash, current_lines.join("\n")));
        }
        current_lines.clear();
    }
}

/// `splitRawSections` — per-section (path, file_hash, diff) triples.
fn split_raw_sections(input: &str) -> Result<Vec<(String, Option<String>, String)>, String> {
    let stripped = strip_leading_blank_lines(input);
    let lines: Vec<&str> = stripped.split('\n').collect();
    let first_line = lines.first().copied().unwrap_or("");

    if parse_hashline_header_line(first_line)?.is_none() {
        let first_trimmed = first_line.trim_end();
        let unified_header = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"^@@\s+[-+]?\d+,\d+\s+[-+]?\d+,\d+\s+@@")
                .expect("static unified regex") // safety: hardcoded compile-time regex literal
        });
        if unified_header.is_match(first_trimmed) {
            return Err(
                "unified-diff hunk header (`@@ -N,M +N,M @@`) is not valid in hashline. File sections start with `[path#HASH]`; use `replace`, `delete`, or `insert` ops.".to_string(),
            );
        }
        let preview = if first_line.len() > 120 {
            &first_line[..first_line.floor_char_boundary(120)]
        } else {
            first_line
        };
        return Err(format!(
            "input must begin with \"{HL_FILE_PREFIX}PATH{HL_FILE_HASH_SEP}HASH{HL_FILE_SUFFIX}\" on the first non-blank line for anchored edits; got: {}. Example: \"{HL_FILE_PREFIX}src/foo.ts{HL_FILE_HASH_SEP}{}{HL_FILE_SUFFIX}\" then edit ops.",
            serde_json::to_string(preview).unwrap_or_default(),
            HL_FILE_HASH_EXAMPLES[0]
        ));
    }

    let mut sections: Vec<(String, Option<String>, String)> = Vec::new();
    let mut current: Option<(String, Option<String>)> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim_end();
        let token = classify_line(line, 0);
        if matches!(token, Token::EnvelopeEnd { .. } | Token::Abort { .. }) {
            break;
        }
        if matches!(token, Token::EnvelopeBegin { .. }) {
            continue;
        }
        if trimmed.starts_with(HL_FILE_PREFIX)
            && let Some(header) = parse_hashline_header_line(line)?
        {
            flush_raw_section(&mut sections, &mut current, &mut current_lines);
            current = Some(header);
            continue;
        }
        current_lines.push(line.to_string());
    }
    flush_raw_section(&mut sections, &mut current, &mut current_lines);
    Ok(sections)
}

/// `mergeSamePathSections`: collapse sections targeting the same path.
fn merge_same_path_sections(
    sections: Vec<(String, Option<String>, String)>,
) -> Result<Vec<(String, Option<String>, String)>, String> {
    let mut merged: Vec<(String, Option<String>, String)> = Vec::new();
    for (path, file_hash, diff) in sections {
        if let Some(existing) = merged
            .iter_mut()
            .find(|(existing_path, ..)| *existing_path == path)
        {
            if existing.1.is_some() && file_hash.is_some() && existing.1 != file_hash {
                return Err(format!(
                    "Conflicting hashline snapshot tags for {path}: #{} and #{}. Re-read the file and retry with one current header.",
                    existing.1.as_deref().unwrap_or_default(),
                    file_hash.as_deref().unwrap_or_default()
                ));
            }
            if existing.1.is_none() && file_hash.is_some() {
                existing.1 = file_hash;
            }
            existing.2.push('\n');
            existing.2.push_str(&diff);
        } else {
            merged.push((path, file_hash, diff));
        }
    }
    Ok(merged)
}

/// A parsed hashline patch section (pinned `PatchSection`).
pub(crate) struct PatchSection {
    pub(crate) path: String,
    pub(crate) file_hash: Option<String>,
    pub(crate) diff: String,
    parsed: std::sync::OnceLock<ParsedPatch>,
}

impl PatchSection {
    fn new(path: String, file_hash: Option<String>, diff: String) -> Self {
        Self {
            path,
            file_hash,
            diff,
            parsed: std::sync::OnceLock::new(),
        }
    }

    pub(crate) fn clone_from_ref(&self) -> PatchSection {
        PatchSection::new(self.path.clone(), self.file_hash.clone(), self.diff.clone())
    }

    pub(crate) fn parse(&self) -> Result<&ParsedPatch, String> {
        if let Some(parsed) = self.parsed.get() {
            return Ok(parsed);
        }
        let parsed = parse_patch(&self.diff)?;
        // The cell was empty above and `set` only fails on a concurrent
        // writer, which still leaves the cell initialized, so `get` below
        // is guaranteed Some; fail closed on the parse-error path rather
        // than panicking.
        let _ = self.parsed.set(parsed);
        match self.parsed.get() {
            Some(parsed) => Ok(parsed),
            None => Err("internal: parsed patch cell was not initialized".to_string()),
        }
    }

    pub(crate) fn edits(&self) -> Result<&[Edit], String> {
        Ok(&self.parse()?.0)
    }

    pub(crate) fn file_op(&self) -> Result<Option<&FileOp>, String> {
        Ok(self.parse()?.1.as_ref())
    }

    pub(crate) fn warnings(&self) -> Result<&[String], String> {
        Ok(&self.parse()?.2)
    }

    pub(crate) fn has_anchor_scoped_edit(&self) -> Result<bool, String> {
        Ok(self.edits()?.iter().any(edit_has_anchor_scope))
    }

    pub(crate) fn collect_anchor_lines(&self) -> Result<Vec<u64>, String> {
        let mut lines: Vec<u64> = Vec::new();
        for edit in self.edits()? {
            match edit {
                Edit::Delete { anchor, .. } | Edit::Block { anchor, .. } => lines.push(anchor.line),
                Edit::Cut { range, .. } => {
                    for line in range.start.line..=range.end.line {
                        lines.push(line);
                    }
                }
                Edit::Paste { at, .. } => match at {
                    PasteTarget::Span(range) => {
                        for line in range.start.line..=range.end.line {
                            lines.push(line);
                        }
                    }
                    PasteTarget::Gap(Cursor::BeforeAnchor(anchor))
                    | PasteTarget::Gap(Cursor::AfterAnchor(anchor)) => lines.push(anchor.line),
                    PasteTarget::Gap(_) => {}
                },
                Edit::Insert { cursor, .. } => {
                    if let Cursor::BeforeAnchor(anchor) | Cursor::AfterAnchor(anchor) = cursor {
                        lines.push(anchor.line);
                    }
                }
            }
        }
        lines.sort_unstable();
        lines.dedup();
        Ok(lines)
    }

    /// Apply this section's edits to `text` (pure; no tag validation).
    pub(crate) fn apply_to(
        &self,
        text: &str,
        clipboard: &mut Clipboard,
    ) -> Result<ApplyResult, String> {
        let (edits, _, warnings) = self.parse()?.clone();
        let mut resolve_warnings: Vec<String> = Vec::new();
        let mut resolutions: Vec<BlockResolution> = Vec::new();
        let resolved = resolve_block_edits(
            &edits,
            text,
            OnUnresolved::Throw,
            &mut resolve_warnings,
            &mut resolutions,
        )?;
        let mut result = apply_edits(text, &resolved, clipboard)?;
        let mut merged = warnings.clone();
        merged.extend(resolve_warnings);
        merged.append(&mut result.warnings);
        result.warnings = merged;
        result.block_resolutions = resolutions;
        Ok(result)
    }
}

fn edit_has_anchor_scope(edit: &Edit) -> bool {
    match edit {
        Edit::Delete { .. } | Edit::Block { .. } | Edit::Cut { .. } => true,
        Edit::Paste { at, .. } => match at {
            PasteTarget::Span(_) => true,
            PasteTarget::Gap(Cursor::BeforeAnchor(_))
            | PasteTarget::Gap(Cursor::AfterAnchor(_)) => true,
            PasteTarget::Gap(_) => false,
        },
        Edit::Insert { cursor, .. } => {
            matches!(cursor, Cursor::BeforeAnchor(_) | Cursor::AfterAnchor(_))
        }
    }
}

/// A parsed hashline patch (pinned `Patch`).
pub(crate) struct Patch {
    pub(crate) sections: Vec<PatchSection>,
}

impl Patch {
    pub(crate) fn parse(input: &str) -> Result<Patch, String> {
        let raw = split_raw_sections(input)?;
        let merged = merge_same_path_sections(raw)?;
        Ok(Patch {
            sections: merged
                .into_iter()
                .map(|(path, file_hash, diff)| PatchSection::new(path, file_hash, diff))
                .collect(),
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// diff + compact preview — pinned `edit/diff.ts` (generateDiffString shape)
// and `packages/hashline/src/diff-preview.ts`
// ═══════════════════════════════════════════════════════════════════════════

/// `generateDiffString` port: Myers line diff emitting `+N|c` / `-N|c` /
/// ` N|c` rows with `contextLines` of context around changes.
fn generate_diff_string(old: &str, new: &str, context_lines: usize) -> (String, Option<u64>) {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut output: Vec<String> = Vec::new();
    let mut old_line_num: u64 = 1;
    let mut new_line_num: u64 = 1;
    let mut first_changed_line: Option<u64> = None;
    let mut last_was_change = false;

    let mut parts: Vec<(similar::ChangeTag, Vec<&str>)> = Vec::new();
    for change in diff.iter_all_changes() {
        let text = change.value().strip_suffix('\n').unwrap_or(change.value());
        match parts.last_mut() {
            Some((tag, lines)) if *tag == change.tag() => lines.push(text),
            _ => parts.push((change.tag(), vec![text])),
        }
    }

    for (index, (tag, lines)) in parts.iter().enumerate() {
        match tag {
            similar::ChangeTag::Equal => {
                let next_is_change = parts
                    .get(index + 1)
                    .is_some_and(|(next_tag, _)| *next_tag != similar::ChangeTag::Equal);
                if last_was_change || next_is_change {
                    let context_limit = context_lines;
                    let mut lines_to_show: Vec<&str>;
                    if last_was_change && next_is_change {
                        if lines.len() > context_limit * 2 {
                            lines_to_show = lines[..context_limit].to_vec();
                            lines_to_show.extend_from_slice(&lines[lines.len() - context_limit..]);
                        } else {
                            lines_to_show = lines.clone();
                        }
                    } else if next_is_change {
                        let leading_skip = lines.len().saturating_sub(context_limit);
                        lines_to_show = lines[leading_skip..].to_vec();
                    } else {
                        lines_to_show = lines[..lines.len().min(context_limit)].to_vec();
                    }
                    for line in &lines_to_show {
                        output.push(format!(" {new_line_num}|{line}"));
                        new_line_num += 1;
                        old_line_num += 1;
                    }
                } else {
                    new_line_num += lines.len() as u64;
                    old_line_num += lines.len() as u64;
                }
                last_was_change = false;
            }
            similar::ChangeTag::Delete => {
                if first_changed_line.is_none() {
                    first_changed_line = Some(new_line_num);
                }
                for line in lines {
                    output.push(format!("-{old_line_num}|{line}"));
                    old_line_num += 1;
                }
                last_was_change = true;
            }
            similar::ChangeTag::Insert => {
                if first_changed_line.is_none() {
                    first_changed_line = Some(new_line_num);
                }
                for line in lines {
                    output.push(format!("+{new_line_num}|{line}"));
                    new_line_num += 1;
                }
                last_was_change = true;
            }
        }
    }

    (output.join("\n"), first_changed_line)
}

/// `buildCompactDiffPreview` from the pinned `diff-preview.ts`.
fn build_compact_diff_preview(diff: &str) -> String {
    const PREVIEW_ELISION_MARKER: &str = "…";
    const DEFAULT_ADDED_RUN_CONTEXT_LINES: usize = 2;

    let lines: Vec<&str> = if diff.is_empty() {
        Vec::new()
    } else {
        diff.split('\n').collect()
    };
    let mut added_lines = 0usize;
    let mut removed_lines = 0usize;
    let mut formatted: Vec<String> = Vec::new();
    let mut added_run: Vec<String> = Vec::new();

    fn is_preview_separator(line: &str) -> bool {
        line == PREVIEW_ELISION_MARKER || line.is_empty()
    }

    fn append_preview_line(output: &mut Vec<String>, line: String) {
        let normalized = if line == "..." || line == PREVIEW_ELISION_MARKER || line == "+…" {
            PREVIEW_ELISION_MARKER.to_string()
        } else {
            line
        };
        if is_preview_separator(&normalized)
            && (output.is_empty()
                || is_preview_separator(output.last().map(String::as_str).unwrap_or("")))
        {
            return;
        }
        output.push(normalized);
    }

    fn append_added_run(formatted: &mut Vec<String>, run: &mut Vec<String>) {
        if run.is_empty() {
            return;
        }
        let collapse_threshold = DEFAULT_ADDED_RUN_CONTEXT_LINES * 2 + 1;
        if run.len() <= collapse_threshold {
            for text in run.iter() {
                append_preview_line(formatted, text.clone());
            }
        } else {
            for text in run.iter().take(DEFAULT_ADDED_RUN_CONTEXT_LINES) {
                append_preview_line(formatted, text.clone());
            }
            append_preview_line(formatted, PREVIEW_ELISION_MARKER.to_string());
            for text in run.iter().skip(run.len() - DEFAULT_ADDED_RUN_CONTEXT_LINES) {
                append_preview_line(formatted, text.clone());
            }
        }
        // Pinned `flushAddedRun` resets the buffer (`addedRun.length = 0`);
        // without this the run would be re-emitted by every later flush.
        run.clear();
    }

    for line in lines {
        match parse_numbered_diff_line(line) {
            None => {
                append_added_run(&mut formatted, &mut added_run);
                append_preview_line(&mut formatted, line.to_string());
            }
            Some((kind, line_number, content)) => match kind {
                '+' => {
                    added_lines += 1;
                    added_run.push(format!("{line_number}:{content}"));
                }
                '-' => {
                    append_added_run(&mut formatted, &mut added_run);
                    removed_lines += 1;
                }
                _ => {
                    append_added_run(&mut formatted, &mut added_run);
                    let new_line_number = line_number + added_lines as i64 - removed_lines as i64;
                    append_preview_line(&mut formatted, format!("{new_line_number}:{content}"));
                }
            },
        }
    }
    append_added_run(&mut formatted, &mut added_run);
    while formatted
        .last()
        .is_some_and(|line| is_preview_separator(line))
    {
        formatted.pop();
    }
    formatted.join("\n")
}

fn parse_numbered_diff_line(line: &str) -> Option<(char, i64, &str)> {
    let kind = line.chars().next()?;
    if kind != '+' && kind != '-' && kind != ' ' {
        return None;
    }
    let body = &line[1..];
    let sep = body.find('|')?;
    let line_number: i64 = body[..sep].parse().ok()?;
    Some((kind, line_number, &body[sep + 1..]))
}

// ═══════════════════════════════════════════════════════════════════════════
// patcher + engine entry — pinned `packages/hashline/src/patcher.ts` +
// `packages/coding-agent/src/edit/hashline/execute.ts` (subset)
// ═══════════════════════════════════════════════════════════════════════════

struct PreparedSection {
    section: PatchSection,
    virtual_path: ironclaw_host_api::path::VirtualPath,
    exists: bool,
    normalized: String,
    apply_result: ApplyResult,
    parse_warnings: Vec<String>,
    file_op: Option<FileOp>,
    version: Option<ironclaw_filesystem::RecordVersion>,
}

impl PreparedSection {
    fn is_noop(&self) -> bool {
        self.file_op.is_none() && self.apply_result.text == self.normalized
    }
}

fn detect_line_ending(content: &str) -> LineEnding {
    let crlf = content.find("\r\n");
    let lf = content.find('\n');
    match (crlf, lf) {
        (Some(crlf), Some(lf)) => {
            if crlf < lf {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            }
        }
        _ => LineEnding::Lf,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    CrLf,
}

pub(crate) fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn restore_line_endings(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::CrLf => text.replace('\n', "\r\n"),
    }
}

/// Classify a hashline parse/apply failure message into the specific engine
/// kind at the boundary where the string becomes a `CodingEngineError`. The
/// messages are pinned-verbatim, so their leading sentences identify the
/// originating site unambiguously: bounds validation (`validateLineBounds`),
/// absolute-range validation (`validateRange`), and the malformed line
/// reference (`parseTag`).
fn classify_hashline_error(message: &str) -> CodingEngineErrorKind {
    if let Some(rest) = message.strip_prefix("Line ")
        && rest.contains(" does not exist (file has ")
    {
        return CodingEngineErrorKind::LineOutOfBounds;
    }
    if message.starts_with("line ") && message.contains(": Invalid absolute range: ") {
        return CodingEngineErrorKind::InvalidAbsoluteRange;
    }
    if message.starts_with("Invalid line reference.") {
        return CodingEngineErrorKind::MalformedLineReference;
    }
    CodingEngineErrorKind::HashlineApply
}

fn hashline_parse_error(message: String) -> CodingEngineError {
    let kind = classify_hashline_error(&message);
    coding_error(kind, message)
}

fn file_not_found_message(path: &str) -> CodingEngineError {
    coding_error(
        CodingEngineErrorKind::PathNotFound,
        format!("File not found: {path}. Use the write tool to create new files."),
    )
}

fn mismatch_error(
    ctx: &CodingEngineContext,
    scope_key: &CodingScopeKey,
    virtual_path: &ironclaw_host_api::path::VirtualPath,
    section_path: &str,
    expected: &str,
    normalized: &str,
    section: &PatchSection,
) -> CodingEngineError {
    let actual = compute_file_hash(normalized);
    let anchor_lines = section.collect_anchor_lines().unwrap_or_default();
    let recognized = ctx
        .snapshots
        .tag_recognized(scope_key, virtual_path.as_str(), expected);
    let file_lines: Vec<String> = normalized.split('\n').map(ToString::to_string).collect();
    let message = render_mismatch_message(
        Some(section_path),
        expected,
        &actual,
        &file_lines,
        &anchor_lines,
        recognized,
    );
    coding_error(super::state::stale_anchor_kind(recognized), message)
}

/// Read a section's target file, validate the snapshot tag, apply edits in
/// memory (mirrors `Patcher.prepare`; NO fuzzy recovery — a drifted file is
/// rejected with the exact MismatchError).
async fn prepare_section(
    ctx: &CodingEngineContext,
    scope_key: &CodingScopeKey,
    section: PatchSection,
) -> Result<PreparedSection, CodingEngineError> {
    let resolved = resolve_input_path(ctx, &section.path, FilesystemOperation::WriteFile)
        .map_err(|_| file_not_found_message(&section.path))?;
    let virtual_path = resolved.virtual_path;

    let stat = ctx.filesystem.stat(&virtual_path).await.map_err(|error| {
        if matches!(error, ironclaw_filesystem::FilesystemError::NotFound { .. }) {
            file_not_found_message(&section.path)
        } else {
            coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            )
        }
    })?;
    if stat.sensitive {
        return Err(super::filesystem_denied());
    }

    let Some(expected) = section.file_hash.clone() else {
        return Err(coding_error(
            CodingEngineErrorKind::HashlineApply,
            missing_snapshot_tag_message(&section.path),
        ));
    };

    let (exists, raw_content, version) = match ctx.filesystem.get(&virtual_path).await {
        Ok(Some(versioned)) => (true, versioned.entry.body, Some(versioned.version)),
        Ok(None) => (false, Vec::new(), None),
        Err(error) => {
            return Err(coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            ));
        }
    };
    if !exists {
        return Err(file_not_found_message(&section.path));
    }

    let text = String::from_utf8(raw_content).map_err(|_| file_not_found_message(&section.path))?;
    let normalized = normalize_to_lf(&text);
    let live_matches =
        ctx.snapshots
            .snapshot_matches(scope_key, virtual_path.as_str(), &expected, &normalized);

    // Run-isolation gate (IronClaw-specific; see `state.rs`): the snapshot
    // registry binds tags to the scope+run that recorded them, so a tag must
    // have been recorded in THIS run even when the live content happens to
    // hash to it. The pinned source trusts content-derived tags outright
    // (`#applyWithRecovery`: "when the live text hashes to it, trust the
    // match and apply directly"); the run dimension is an IronClaw addition
    // and rejects a stale tag from a prior run before any apply path.
    if !ctx
        .snapshots
        .tag_recognized(scope_key, virtual_path.as_str(), &expected)
    {
        return Err(mismatch_error(
            ctx,
            scope_key,
            &virtual_path,
            &section.path,
            &expected,
            &normalized,
            &section,
        ));
    }

    let parse_warnings = section.warnings().map_err(hashline_parse_error)?.to_vec();
    let file_op = section.file_op().map_err(hashline_parse_error)?.cloned();
    if file_op.is_some() {
        resolve_input_path(ctx, &section.path, FilesystemOperation::Delete)
            .map_err(|_| super::filesystem_denied())?;
    }

    // Block edits resolve against the tagged snapshot text; a drifted file is
    // rejected before resolution (no fuzzy writes).
    let has_block_edit = section
        .edits()
        .map_err(hashline_parse_error)?
        .iter()
        .any(|edit| matches!(edit, Edit::Block { .. }));

    if !live_matches {
        if has_block_edit {
            return Err(mismatch_error(
                ctx,
                scope_key,
                &virtual_path,
                &section.path,
                &expected,
                &normalized,
                &section,
            ));
        }
        // Head/tail-only inserts are position-stable: apply onto live content
        // with the drift warning instead of rejecting.
        if !section
            .has_anchor_scoped_edit()
            .map_err(hashline_parse_error)?
        {
            let mut clipboard = Clipboard::default();
            let (edits, _, _) = section.parse().map_err(hashline_parse_error)?.clone();
            let mut result =
                apply_edits(&normalized, &edits, &mut clipboard).map_err(hashline_parse_error)?;
            let mut warnings = parse_warnings.clone();
            warnings.insert(0, HEADTAIL_DRIFT_WARNING.to_string());
            warnings.append(&mut result.warnings);
            result.warnings = warnings;
            return Ok(PreparedSection {
                section,
                virtual_path,
                exists,
                normalized,
                apply_result: result,
                parse_warnings,
                file_op,
                version,
            });
        }
        return Err(mismatch_error(
            ctx,
            scope_key,
            &virtual_path,
            &section.path,
            &expected,
            &normalized,
            &section,
        ));
    }

    // Tag matches live content: apply directly (no fuzzy writes). `apply_to`
    // merges parse + block-resolution + apply warnings and carries the block
    // resolutions onto the result.
    let mut clipboard = Clipboard::default();
    let result = section
        .apply_to(&normalized, &mut clipboard)
        .map_err(hashline_parse_error)?;

    Ok(PreparedSection {
        section,
        virtual_path,
        exists,
        normalized,
        apply_result: result,
        parse_warnings,
        file_op,
        version,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionOp {
    Create,
    Update,
    Delete,
    Noop,
}

struct SectionResult {
    op: SectionOp,
    path: String,
    header: String,
    before: String,
    after: String,
    warnings: Vec<String>,
    move_dest: Option<String>,
    block_resolutions: Vec<BlockResolution>,
}

/// Commit a prepared section to the backend with a CAS write (mirrors
/// `Patcher.commit` plus the slice contract: `put` with
/// [`CasExpectation::Version`]).
async fn commit_section(
    ctx: &CodingEngineContext,
    scope_key: &CodingScopeKey,
    prepared: PreparedSection,
) -> Result<SectionResult, CodingEngineError> {
    let PreparedSection {
        section,
        virtual_path,
        exists,
        normalized,
        apply_result,
        parse_warnings,
        file_op,
        version,
    } = prepared;
    let after = apply_result.text;
    let mut warnings = parse_warnings.clone();
    warnings.extend(apply_result.warnings.iter().cloned());
    let move_dest = match &file_op {
        Some(FileOp::Move { dest }) => Some(dest.clone()),
        _ => None,
    };
    let result_path = move_dest.clone().unwrap_or_else(|| section.path.clone());

    if matches!(file_op, Some(FileOp::Rem)) {
        ctx.filesystem
            .delete(&virtual_path)
            .await
            .map_err(|error| {
                coding_error(
                    CodingEngineErrorKind::Filesystem,
                    format!("filesystem error: {error}"),
                )
            })?;
        ctx.snapshots.invalidate(scope_key, virtual_path.as_str());
        let hash = compute_file_hash(&normalized);
        return Ok(SectionResult {
            op: SectionOp::Delete,
            path: section.path.clone(),
            header: format_hashline_header(&section.path, &hash),
            before: normalized.clone(),
            after: normalized,
            warnings,
            move_dest: None,
            block_resolutions: Vec::new(),
        });
    }

    if after == normalized && move_dest.is_none() {
        let hash = ctx
            .snapshots
            .record_and_return(scope_key, virtual_path.as_str(), &normalized);
        return Ok(SectionResult {
            op: SectionOp::Noop,
            path: section.path.clone(),
            header: format_hashline_header(&section.path, &hash),
            before: normalized.clone(),
            after: normalized,
            warnings,
            move_dest: None,
            block_resolutions: Vec::new(),
        });
    }

    if let Some(dest) = &move_dest {
        let dest_resolved = resolve_input_path(ctx, dest, FilesystemOperation::WriteFile)
            .map_err(|_| file_not_found_message(dest))?;
        let dest_path = dest_resolved.virtual_path;
        // Parent directories are established implicitly by `put` (see
        // `write.rs`; `create_dir_all` is unimplemented on in-memory
        // backends).
        let persisted = restore_line_endings(&after, detect_line_ending(&after));
        ctx.filesystem
            .put(
                &dest_path,
                Entry::bytes(persisted.into_bytes()),
                CasExpectation::Any,
            )
            .await
            .map_err(|error| {
                coding_error(
                    CodingEngineErrorKind::Filesystem,
                    format!("filesystem error: {error}"),
                )
            })?;
        ctx.filesystem
            .delete(&virtual_path)
            .await
            .map_err(|error| {
                coding_error(
                    CodingEngineErrorKind::Filesystem,
                    format!("filesystem error: {error}"),
                )
            })?;
        ctx.snapshots.invalidate(scope_key, virtual_path.as_str());
        let hash = ctx
            .snapshots
            .record_and_return(scope_key, dest_path.as_str(), &after);
        return Ok(SectionResult {
            op: SectionOp::Update,
            path: result_path,
            header: format_hashline_header(dest, &hash),
            before: normalized,
            after,
            warnings,
            move_dest: Some(dest.clone()),
            block_resolutions: apply_result.block_resolutions.clone(),
        });
    }

    let persisted = restore_line_endings(&after, detect_line_ending(&after));
    let cas = match (exists, version) {
        (true, Some(version)) => CasExpectation::Version(version),
        _ => CasExpectation::Any,
    };
    let cas_result = ctx
        .filesystem
        .put(
            &virtual_path,
            Entry::bytes(persisted.clone().into_bytes()),
            cas,
        )
        .await;
    match cas_result {
        Ok(_) => {}
        // Byte-only backends (disk/in-memory) cannot honor a version CAS; the
        // tag/live-hash validation in `prepare_section` already guards against
        // mid-air collisions, so fall back to the unconditional write exactly
        // like the `write` engine and the stock `apply_patch` builtin (which
        // CAS-free via `write_file`). Production record backends keep the CAS.
        Err(ironclaw_filesystem::FilesystemError::Unsupported {
            operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
            ..
        }) => {
            ctx.filesystem
                .put(
                    &virtual_path,
                    Entry::bytes(persisted.clone().into_bytes()),
                    CasExpectation::Any,
                )
                .await
                .map_err(|error| {
                    coding_error(
                        CodingEngineErrorKind::Filesystem,
                        format!("filesystem error: {error}"),
                    )
                })?;
        }
        Err(error) => {
            return Err(coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            ));
        }
    }
    let op = if exists {
        SectionOp::Update
    } else {
        SectionOp::Create
    };
    let hash = ctx
        .snapshots
        .record_and_return(scope_key, virtual_path.as_str(), &after);
    Ok(SectionResult {
        op,
        path: section.path.clone(),
        header: format_hashline_header(&section.path, &hash),
        before: normalized,
        after,
        warnings,
        move_dest: None,
        block_resolutions: apply_result.block_resolutions,
    })
}

/// `noChangeDiagnostic` from `edit/hashline/execute.ts`.
fn no_change_diagnostic(path: &str) -> String {
    format!(
        "Edits to {path} parsed and applied cleanly, but produced no change: your body row(s) are byte-identical to the file at the targeted lines. The bug is somewhere else — re-read the file before issuing another edit. Do NOT widen the payload or add lines; verify the anchor first."
    )
}

const BLOCK_OP_LABELS: [&str; 4] = ["PUT N*:", "PUT >N*:", "CUT N*", "PUT >N*"];

/// `formatBlockResolution` from `edit/hashline/execute.ts`.
fn format_block_resolution(resolution: &BlockResolution) -> String {
    let label = match resolution.op {
        BlockOp::Replace => BLOCK_OP_LABELS[0],
        BlockOp::InsertAfter => BLOCK_OP_LABELS[1],
        BlockOp::Cut => BLOCK_OP_LABELS[2],
        BlockOp::PasteAfter => BLOCK_OP_LABELS[3],
    };
    let op = label.replace('N', &resolution.anchor_line.to_string());
    let lines = resolution.end - resolution.start + 1;
    let span = if resolution.start == resolution.end {
        format!("line {}", resolution.start)
    } else {
        format!("lines {}-{}", resolution.start, resolution.end)
    };
    let suffix = match resolution.op {
        BlockOp::InsertAfter => format!("; body lands after line {}", resolution.end),
        BlockOp::PasteAfter => format!("; clipboard lands after line {}", resolution.end),
        _ => String::new(),
    };
    let plural = if lines == 1 { "" } else { "s" };
    format!("{op} → resolved {span} ({lines} line{plural}){suffix}")
}

/// Render one section result into the model-visible text (`renderSection` in
/// the pinned `execute.ts`).
fn render_section(result: &SectionResult) -> String {
    match result.op {
        SectionOp::Delete => format!("Deleted {}", result.path),
        SectionOp::Noop => no_change_diagnostic(&result.path),
        _ => {
            let (diff, _) = generate_diff_string(&result.before, &result.after, 2);
            let preview = build_compact_diff_preview(&diff);
            let warnings_block = if result.warnings.is_empty() {
                String::new()
            } else {
                format!("\n\nWarnings:\n{}", result.warnings.join("\n"))
            };
            let preview_block = if preview.is_empty() {
                String::new()
            } else {
                format!("\n{preview}")
            };
            let block_block = if result.block_resolutions.is_empty() {
                String::new()
            } else {
                let lines: Vec<String> = result
                    .block_resolutions
                    .iter()
                    .map(format_block_resolution)
                    .collect();
                format!("\n{}", lines.join("\n"))
            };
            let move_block = result
                .move_dest
                .as_ref()
                .map(|dest| format!("\nMoved to {dest}"))
                .unwrap_or_default();
            format!(
                "{}{block_block}{move_block}{preview_block}{warnings_block}",
                result.header
            )
        }
    }
}

impl CodingSnapshotRegistry {
    /// Record a snapshot and return its tag (computed here so callers never
    /// drift from the pinned normalization).
    pub(crate) fn record_and_return(
        &self,
        scope: &CodingScopeKey,
        virtual_path: &str,
        text: &str,
    ) -> String {
        let tag = compute_file_hash(text);
        self.record(
            scope,
            virtual_path,
            &tag,
            *blake3::hash(text.as_bytes()).as_bytes(),
        );
        tag
    }
}

/// The `edit` engine entry point: parse the hashline input, prepare every
/// section (fail fast), commit each with CAS writes, and render the output
/// (`executeHashlineSingle` semantics; multi-section output joined by blank
/// lines).
pub(crate) async fn edit(
    ctx: &CodingEngineContext,
    input: Value,
) -> Result<String, CodingEngineError> {
    let Some(input_text) = input.get("input").and_then(Value::as_str) else {
        return Err(input_error("edit requires a string `input`"));
    };
    let patch = Patch::parse(input_text).map_err(hashline_parse_error)?;
    if patch.sections.is_empty() {
        return Err(coding_error(
            CodingEngineErrorKind::HashlineApply,
            NO_HASHLINE_SECTIONS,
        ));
    }
    let scope_key = CodingScopeKey::from_scope(&ctx.scope, ctx.run_id);

    // Prepare every section first so any failure surfaces before any write.
    let mut prepared: Vec<PreparedSection> = Vec::with_capacity(patch.sections.len());
    for section in &patch.sections {
        prepared.push(prepare_section(ctx, &scope_key, section.clone_from_ref()).await?);
    }
    // Multi-section no-ops are hard failures (`executeHashlineSingle` throws
    // the no-change diagnostic for them).
    if prepared.len() > 1 {
        for entry in &prepared {
            if entry.is_noop() {
                return Err(coding_error(
                    CodingEngineErrorKind::HashlineApply,
                    no_change_diagnostic(&entry.section.path),
                ));
            }
        }
    }

    let mut rendered: Vec<String> = Vec::with_capacity(prepared.len());
    for entry in prepared {
        let result = commit_section(ctx, &scope_key, entry).await?;
        rendered.push(render_section(&result));
    }
    Ok(rendered.join("\n\n"))
}

/// Aggregate failure shapes from `packages/coding-agent/src/edit/index.ts`
/// (the pinned multi-file / multi-entry wrappers). Rendered here so the
/// templates are reachable through the engine's error surface and the
/// differential seam test can drive them against the golden fixtures.
/// `per_file_failure_aggregate`: first per-file failure inside a multi-file
/// edit batch.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn render_per_file_failure_aggregate(path: &str, error_text: &str) -> String {
    format!("Error editing {path}: {error_text}")
}

/// `files_not_applied`: the trailing entries of a failed multi-file batch.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn render_files_not_applied(skipped_paths: &str) -> String {
    format!(
        "Files NOT applied: {skipped_paths}; re-read the affected files and re-issue only the failed and unapplied files."
    )
}

/// `multi_entry_aggregate_failure`: an entry failed inside a multi-entry
/// batch on one path, followed by the applied/not-applied notes.
#[allow(dead_code)]
pub(crate) fn render_multi_entry_aggregate_failure(
    path: &str,
    entry_index: usize,
    count: usize,
    error_text: &str,
    applied_count: usize,
) -> String {
    let mut text = format!("Error editing {path} (entry {entry_index} of {count}): {error_text}\n");
    if applied_count == 1 {
        text.push_str("Entry 1 was already applied.\n");
    } else {
        text.push_str(&format!(
            "Entries 1-{applied_count} were already applied.\n"
        ));
    }
    let not_applied_count = count - entry_index;
    if not_applied_count == 1 {
        text.push_str(&format!("Entry {count} was NOT applied; re-read the file and re-issue only the failed and unapplied entries."));
    } else {
        text.push_str(&format!(
            "Entries {}-{count} were NOT applied; re-read the file and re-issue only the failed and unapplied entries.",
            entry_index + 1
        ));
    }
    text
}

/// Aggregate shape variants referenced by `render_multi_entry_aggregate_failure`
/// for the golden `files_not_applied` template.
#[allow(dead_code)]
pub(crate) fn render_entry_not_applied(count: usize) -> String {
    format!(
        "Entry {count} was NOT applied; re-read the file and re-issue only the failed and unapplied entries."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_file_hash_matches_pinned_normalization() {
        // Pinned normalization (`[ \t\r]+(?=\n|$)`): trailing [ \t\r] is
        // stripped only immediately before a newline or EOF — per line AND
        // on the final line, but never leading whitespace of a line.
        assert_eq!(compute_file_hash("a\nb\n"), compute_file_hash("a \nb \n"));
        assert_eq!(compute_file_hash("a\nb\n"), compute_file_hash("a\r\nb\r\n"));
        assert_eq!(compute_file_hash("x"), compute_file_hash("x "));
        assert_eq!(compute_file_hash("x"), compute_file_hash("x\t"));
        assert_eq!(compute_file_hash("x\n"), compute_file_hash("x \n"));
        assert_eq!(compute_file_hash(" \n"), compute_file_hash("\t\n"));
        // Leading whitespace of a line is NOT stripped: "a \n\tb\r\n"
        // normalizes to "a\n\tb\n", which hashes differently from "a\nb\n".
        assert_ne!(
            compute_file_hash("a\nb\n"),
            compute_file_hash("a \n\tb\r\n")
        );
        // Different content hashes differently.
        assert_ne!(compute_file_hash("a"), compute_file_hash("b"));
        // Exact tags verified against the pinned computeFileHash
        // (`Bun.hash.xxHash32(normalized, 0) & 0xffff`, 4-hex uppercase).
        assert_eq!(compute_file_hash(""), "5D05");
        assert_eq!(compute_file_hash("x"), "30EA");
        assert_eq!(compute_file_hash("a\nb\n"), "9A46");
        assert_eq!(compute_file_hash("a \n\tb\r\n"), "114A");
        assert_eq!(compute_file_hash("line1\nline2\n"), "165F");
        // Tags are 4 uppercase hex chars.
        let tag = compute_file_hash("line1\nline2\n");
        assert_eq!(tag.len(), 4);
        assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(tag, tag.to_ascii_uppercase());
    }

    #[test]
    fn mismatch_messages_match_golden_templates() {
        let file_lines: Vec<String> = vec![
            "line one".to_string(),
            "line two".to_string(),
            "line three".to_string(),
            "line four".to_string(),
        ];
        let recognized =
            render_mismatch_message(Some("src/foo.ts"), "1A2B", "3C4D", &file_lines, &[1], true);
        assert!(
            recognized.starts_with(
                "Edit rejected for src/foo.ts: file changed between read and edit.\n\
                 Section is bound to #1A2B, but the current file hashes to #3C4D. \
                 If a prior edit in this session modified this file, copy the [path#newhash] header from that edit's response; \
                 otherwise re-read the file with `read` to refresh the tag before retrying."
            ),
            "{recognized}"
        );
        let unrecognized =
            render_mismatch_message(Some("src/foo.ts"), "9F00", "3C4D", &file_lines, &[1], false);
        assert!(
            unrecognized.starts_with(
                "Edit rejected for src/foo.ts: hash #9F00 is not from this session.\n\
                 The current file hashes to #3C4D. Re-read the file with `read` to copy a current [path#tag] header — \
                 never invent the tag and never reuse one from a prior session."
            ),
            "{unrecognized}"
        );
    }

    #[test]
    fn invalid_absolute_range_message_matches_pinned_source() {
        let message =
            invalid_absolute_range_message(3, 10, 15, AbsoluteRangeOp::Replace, None, None);
        assert_eq!(
            message,
            "line 3: Invalid absolute range: start 10, end 15. The value after `.=` is an absolute source line, not a line count or replacement length. For one line use `PUT 10:`. For 15 lines starting at 10, use `PUT 10.=24:`."
        );
    }

    #[test]
    fn line_out_of_bounds_and_malformed_reference() {
        assert_eq!(
            line_out_of_bounds(500, 42),
            "Line 500 does not exist (file has 42 lines)"
        );
        assert_eq!(
            malformed_line_reference("abc"),
            "Invalid line reference. Expected a bare line number from read/search output plus the section header content-hash tag (for example [src/foo.ts#1A2B] and line \"160\") Received \"abc\"."
        );
    }

    #[test]
    fn tokenizer_parses_hunk_headers() {
        let (target, had_colon) = try_parse_hunk_header("PUT 5.=9:").expect("parses");
        assert!(had_colon);
        match target {
            BlockTarget::Replace { range, register } => {
                assert_eq!(range.start.line, 5);
                assert_eq!(range.end.line, 9);
                assert!(register.is_none());
            }
            other => panic!("unexpected target {other:?}"),
        }
        let (target, had_colon) = try_parse_hunk_header("CUT 3 @clip").expect("parses");
        assert!(!had_colon);
        match target {
            BlockTarget::Cut { range, register } => {
                assert_eq!(range.start.line, 3);
                assert_eq!(range.end.line, 3);
                assert_eq!(register.as_deref(), Some("clip"));
            }
            other => panic!("unexpected target {other:?}"),
        }
        let (target, _) = try_parse_hunk_header("PUT >$:").expect("parses");
        assert_eq!(target, BlockTarget::Eof { register: None });
        let (target, _) = try_parse_hunk_header("PUT <1:").expect("parses");
        assert_eq!(target, BlockTarget::Bof { register: None });
        let (target, _) = try_parse_hunk_header("PUT 7*:").expect("parses");
        assert_eq!(
            target,
            BlockTarget::Block {
                anchor: Anchor { line: 7 },
                register: None
            }
        );
        let (target, _) = try_parse_hunk_header("REM").expect("parses");
        assert_eq!(target, BlockTarget::Rem);
        let (target, _) = try_parse_hunk_header("MV src/renamed.ts").expect("parses");
        assert_eq!(
            target,
            BlockTarget::Move {
                dest: "src/renamed.ts".to_string()
            }
        );
        // Lenient range separators recover: `5-9`, `5..9`, and `5.9` all
        // parse (pinned scanRangeSeparator accepts `.` runs).
        assert!(try_parse_hunk_header("PUT 5-9:").is_some());
        assert!(try_parse_hunk_header("PUT 5..9:").is_some());
        let (target, _) = try_parse_hunk_header("PUT 5.9:").expect("lenient dot separator");
        assert_eq!(
            target,
            BlockTarget::Replace {
                range: ParsedRange {
                    start: Anchor { line: 5 },
                    end: Anchor { line: 9 },
                },
                register: None,
            }
        );
        // A malformed header (trailing garbage) returns None.
        assert!(try_parse_hunk_header("PUT 5.").is_none());
        assert!(try_parse_hunk_header("PUT 5.x:").is_none());
    }

    #[test]
    fn parse_patch_put_replacement() {
        let (edits, file_op, warnings) = parse_patch("PUT 2.=3:\n+new1\n+new2\n").expect("parses");
        assert!(file_op.is_none());
        assert!(warnings.is_empty());
        assert_eq!(edits.len(), 4); // 2 inserts + 2 deletes
        let text = "l1\nold2\nold3\nl4\n".to_string();
        let mut clipboard = Clipboard::default();
        let result = apply_edits(&text, &edits, &mut clipboard).expect("applies");
        assert_eq!(result.text, "l1\nnew1\nnew2\nl4\n");
        assert_eq!(result.first_changed_line, Some(2));
    }

    #[test]
    fn parse_patch_insert_after_gap() {
        let (edits, _, _) = parse_patch("PUT >2:\n+inserted\n").expect("parses");
        let mut clipboard = Clipboard::default();
        let result = apply_edits("a\nb\nc\n", &edits, &mut clipboard).expect("applies");
        assert_eq!(result.text, "a\nb\ninserted\nc\n");
    }

    #[test]
    fn parse_patch_cut_and_paste_anonymous() {
        // A `CUT` followed by a coloned `PUT` with an explicit body inserts
        // the literal body rows (pinned parser: gap targets only paste when
        // the header is register-backed or colonless AND bodyless).
        let (edits, _, _) = parse_patch("CUT 2.=2\nPUT >3:\n+paste1\n").expect("parses");
        let mut clipboard = Clipboard::default();
        let result = apply_edits("a\nb\nc\nd\n", &edits, &mut clipboard).expect("applies");
        // CUT captures line 2 (b); the paste1 body lands after line 3.
        assert_eq!(result.text, "a\nc\npaste1\nd\n");
        // The colonless bodyless form is the anonymous paste: no body rows,
        // so the clipboard (b) is inserted after line 3.
        let (edits, _, _) = parse_patch("CUT 2.=2\nPUT >3\n").expect("parses");
        let mut clipboard = Clipboard::default();
        let result = apply_edits("a\nb\nc\nd\n", &edits, &mut clipboard).expect("applies");
        assert_eq!(result.text, "a\nc\nb\nd\n");
    }

    #[test]
    fn parse_patch_bare_body_rows_warn() {
        let (edits, _, warnings) = parse_patch("PUT 1:\nbare row\n").expect("parses");
        assert!(warnings.contains(&BARE_BODY_AUTO_PIPED_WARNING.to_string()));
        assert!(!edits.is_empty());
    }

    #[test]
    fn parse_patch_minus_rows_auto_pipe_bullets_or_reject() {
        // "- old" is MD-bullet-shaped (`- ` + text): auto-piped as literal
        // content with the bullet warning, never rejected.
        let (edits, _, warnings) = parse_patch("PUT 1:\n- old\n").expect("bullet auto-piped");
        assert!(
            warnings.contains(&MINUS_BULLET_AUTO_PIPED_WARNING.to_string()),
            "{warnings:?}"
        );
        assert!(!edits.is_empty());
        // "-old" (no space after the dash) is not bullet-shaped and has no
        // explicit `+` rows: rejected.
        let error = parse_patch("PUT 1:\n-old\n").expect_err("must reject");
        assert!(error.contains(MINUS_ROW_REJECTED), "{error}");
    }

    #[test]
    fn parse_patch_invalid_absolute_range() {
        let error = parse_patch("PUT 10.=5:\n+x\n").expect_err("must reject");
        assert!(
            error.starts_with("line 1: Invalid absolute range: start 10, end 5."),
            "{error}"
        );
    }

    #[test]
    fn line_out_of_bounds_on_apply() {
        let (edits, _, _) = parse_patch("PUT 99:\n+x\n").expect("parses");
        let mut clipboard = Clipboard::default();
        let error = apply_edits("a\nb\n", &edits, &mut clipboard).expect_err("must reject");
        // `split("\n")` on newline-terminated content yields a trailing
        // empty entry; the pinned bounds message counts it.
        assert_eq!(error, "Line 99 does not exist (file has 3 lines)");
    }

    #[test]
    fn block_resolution_labels_match_golden() {
        let resolution = BlockResolution {
            anchor_line: 42,
            start: 42,
            end: 44,
            op: BlockOp::Replace,
        };
        assert_eq!(
            format_block_resolution(&resolution),
            "PUT 42*: → resolved lines 42-44 (3 lines)"
        );
        let resolution = BlockResolution {
            anchor_line: 7,
            start: 7,
            end: 7,
            op: BlockOp::InsertAfter,
        };
        assert_eq!(
            format_block_resolution(&resolution),
            "PUT >7*: → resolved line 7 (1 line); body lands after line 7"
        );
    }

    #[test]
    fn lexical_block_resolver_finds_brace_blocks() {
        let text = "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n";
        assert_eq!(lexical_block_resolver(text, 1), Some((1, 4)));
        // A line that does not open a block resolves to None.
        assert_eq!(lexical_block_resolver(text, 2), None);
        // Nested braces.
        let nested = "fn a() {\n  if x {\n    y();\n  }\n}\n";
        assert_eq!(lexical_block_resolver(nested, 1), Some((1, 5)));
        assert_eq!(lexical_block_resolver(nested, 2), Some((2, 4)));
    }

    #[test]
    fn compact_diff_preview_shape() {
        // -2|old line / +2|new line -> preview drops the - row and emits the
        // post-edit numbered + row.
        let preview = build_compact_diff_preview("-1|old\n+1|new");
        assert_eq!(preview, "1:new");
        // Context rows renumber to post-edit positions.
        let preview = build_compact_diff_preview(" 1|a\n-2|b\n+2|c\n 3|d");
        assert_eq!(preview, "1:a\n2:c\n3:d");
    }

    #[test]
    fn generate_diff_string_numbers_rows() {
        let (diff, first_changed) = generate_diff_string("a\nb\nc\n", "a\nB\nc\n", 2);
        assert_eq!(first_changed, Some(2));
        assert_eq!(diff, " 1|a\n-2|b\n+2|B\n 3|c");
    }

    #[test]
    fn patch_split_merges_same_path_sections() {
        let patch =
            Patch::parse("[a.ts#1A2B]\nPUT 1:\n+x\n\n[a.ts#1A2B]\nPUT 2:\n+y\n").expect("parses");
        assert_eq!(patch.sections.len(), 1);
        assert_eq!(patch.sections[0].path, "a.ts");
        let (edits, _, _) = patch.sections[0].parse().expect("section parses");
        assert_eq!(edits.len(), 4);
    }

    #[test]
    fn conflicting_tags_rejected() {
        let error = match Patch::parse("[a.ts#1A2B]\nPUT 1:\n+x\n\n[a.ts#3C4D]\nPUT 2:\n+y\n") {
            Ok(_) => panic!("must reject"),
            Err(error) => error,
        };
        assert!(
            error.contains("Conflicting hashline snapshot tags for a.ts: #1A2B and #3C4D."),
            "{error}"
        );
    }

    #[test]
    fn strip_hashline_prefixes_only_when_uniform() {
        let lines = vec!["1:alpha".to_string(), "2:beta".to_string()];
        assert_eq!(
            strip_hashline_prefixes(&lines),
            vec!["alpha".to_string(), "beta".to_string()]
        );
        let mixed = vec!["1:alpha".to_string(), "beta".to_string()];
        assert_eq!(strip_hashline_prefixes(&mixed), mixed);
    }

    #[test]
    fn no_change_diagnostic_matches_golden() {
        let text = no_change_diagnostic("foo.ts");
        assert_eq!(
            text,
            "Edits to foo.ts parsed and applied cleanly, but produced no change: your body row(s) are byte-identical to the file at the targeted lines. The bug is somewhere else — re-read the file before issuing another edit. Do NOT widen the payload or add lines; verify the anchor first."
        );
    }

    #[test]
    fn aggregate_shapes_match_golden_templates() {
        assert_eq!(
            render_per_file_failure_aggregate(
                "src/foo.ts",
                "Line 500 does not exist (file has 42 lines)"
            ),
            "Error editing src/foo.ts: Line 500 does not exist (file has 42 lines)"
        );
        assert_eq!(
            render_files_not_applied("src/a.ts, src/b.ts"),
            "Files NOT applied: src/a.ts, src/b.ts; re-read the affected files and re-issue only the failed and unapplied files."
        );
        let aggregate = render_multi_entry_aggregate_failure(
            "foo.ts",
            2,
            3,
            "Line 500 does not exist (file has 42 lines)",
            1,
        );
        assert_eq!(
            aggregate,
            "Error editing foo.ts (entry 2 of 3): Line 500 does not exist (file has 42 lines)\nEntry 1 was already applied.\nEntry 3 was NOT applied; re-read the file and re-issue only the failed and unapplied entries."
        );
        assert_eq!(
            render_entry_not_applied(3),
            "Entry 3 was NOT applied; re-read the file and re-issue only the failed and unapplied entries."
        );
    }
}
