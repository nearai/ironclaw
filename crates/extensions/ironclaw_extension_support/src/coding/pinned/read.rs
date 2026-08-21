//! `read` engine — files, directories, and scoped `artifact://` results,
//! ported from the pinned `packages/coding-agent/src/tools/read.ts` (disk
//! path), `read-format.ts`, `read-selector.ts`,
//! `session/streaming-output.ts`, and `workspace-tree.ts` at commit
//! `08819b279cf02ae2545e69dad7111ab48d91d35e`.
//!
//! The engine always runs in hashline display mode: file reads emit a
//! `[basename#TAG]` header, `LINE:TEXT` numbered rows, and the pinned
//! truncation/elision notices. Archives, SQLite, documents, URLs, and SSH
//! remain later slices.
//!
//! Documented deviation: the pinned upstream appends the elision footer
//! (`[…Nln elided; re-read needed ranges with <path>:<selector>]`,
//! `formatSummaryElisionFooter`) only in the LLM-summarize path, which is
//! out of scope for this slice; this engine emits the identical footer
//! directly on multi-range reads whose visible spans elide file lines.

use std::time::SystemTime;

use ironclaw_filesystem::{FileType, FilesystemError, FilesystemOperation};
use ironclaw_host_api::artifact::{
    ArtifactAccessError, ArtifactByteRange, ArtifactLineRange, ArtifactReadTarget, ArtifactRef,
    ArtifactSelector,
};
use serde_json::Value;

use super::super::config::{MAX_READ_SIZE, MAX_VISITED_ENTRIES};
use super::hashline::format::{
    format_hashline_header, format_numbered_line, format_numbered_lines,
};
use super::selector::{ParsedSelector, parse_sel, sel_to_offset_limit};
use super::state::CodingScopeKey;
use super::{
    CodingEngineContext, CodingEngineError, CodingEngineErrorKind, coding_error, display_path,
    filesystem_denied, input_error, read_limit_exceeded, resolve_input_path,
    workspace_virtual_root,
};

/// Pinned `DEFAULT_MAX_LINES` (session/streaming-output.ts).
pub(super) const DEFAULT_MAX_LINES: u64 = 3000;
/// Pinned `DEFAULT_MAX_BYTES` (session/streaming-output.ts: 50KB).
pub(super) const DEFAULT_MAX_BYTES: usize = 50 * 1024;
/// `read.defaultLimit` for the issue #7392 target context: the rendered
/// read prompt pins `DEFAULT_LIMIT: "3000"` (read.defaultLimit unset ->
/// DEFAULT_MAX_LINES).
const DEFAULT_LIMIT: u64 = 3000;
/// Assumed bytes per line when scaling the byte budget with the line limit.
pub(super) const BYTES_PER_LINE_BUDGET: usize = 512;

/// `formatBytes` from the pinned `packages/utils/src/format.ts`.
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1}GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// `formatAge` from the pinned `packages/utils/src/format.ts`.
fn format_age(age_seconds: u64) -> String {
    if age_seconds == 0 {
        return String::new();
    }
    let mins = age_seconds / 60;
    let hours = mins / 60;
    let days = hours / 24;
    let weeks = days / 7;
    let months = days / 30;
    if months > 0 {
        format!("{months}mo ago")
    } else if weeks > 0 {
        format!("{weeks}w ago")
    } else if days > 0 {
        format!("{days}d ago")
    } else if hours > 0 {
        format!("{hours}h ago")
    } else if mins > 0 {
        format!("{mins}m ago")
    } else {
        "just now".to_string()
    }
}

/// Split `<path>:<selector>` — `splitPathAndSel` from the pinned
/// `path-utils.ts` (filesystem paths; internal URLs are later slices).
/// Compound trailing selectors are joined (`50-100:raw` / `raw:50-100`) so
/// `parse_sel` sees the full compound; single-component peels are intact.
/// Returns owned strings because a joined compound does not exist as a
/// substring of `raw_path`.
fn split_path_and_sel(raw_path: &str) -> (String, Option<String>) {
    let Some(colon) = raw_path.rfind(':') else {
        return (raw_path.to_string(), None);
    };
    if colon == 0 {
        return (raw_path.to_string(), None);
    }
    let candidate = &raw_path[colon + 1..];
    if !file_line_range_re(candidate) {
        return (raw_path.to_string(), None);
    }
    let mut base_path = &raw_path[..colon];
    let sel = candidate;

    // Compound trailing selector: `path:1-50:raw` or `path:raw:1-50`.
    if let Some(inner_colon) = base_path.rfind(':')
        && inner_colon > 0
    {
        let inner_candidate = &base_path[inner_colon + 1..];
        let inner_is_raw = raw_only_re(inner_candidate);
        let outer_is_raw = raw_only_re(candidate);
        let inner_is_range = line_range_only_re(inner_candidate);
        let outer_is_range = line_range_only_re(candidate);
        if (inner_is_raw && outer_is_range) || (inner_is_range && outer_is_raw) {
            let inner = &base_path[inner_colon + 1..];
            base_path = &base_path[..inner_colon];
            let joined = format!("{inner}:{candidate}");
            return (base_path.to_string(), Some(joined));
        }
    }
    (base_path.to_string(), Some(sel.to_string()))
}

/// `probeLiteralPathExists` for virtual backends: `true` when the exact
/// entry named by `raw_path` exists, or when its existence cannot be ruled
/// out (pinned: `"exists"` and `"unknown"` both win over selector
/// interpretation); only a definitive `NotFound` falls back to the strict
/// split.
async fn literal_path_exists(
    ctx: &CodingEngineContext,
    raw_path: &str,
) -> Result<bool, CodingEngineError> {
    let Ok(resolved) = resolve_input_path(ctx, raw_path, FilesystemOperation::ReadFile) else {
        return Ok(false);
    };
    match ctx.filesystem.stat(&resolved.virtual_path).await {
        Ok(_) => Ok(true),
        Err(FilesystemError::NotFound { .. }) => Ok(false),
        Err(_) => Ok(true),
    }
}

/// FILE_LINE_RANGE_RE: `^(?:<range list>|raw|conflicts)$` (case-insensitive).
fn file_line_range_re(candidate: &str) -> bool {
    if candidate.eq_ignore_ascii_case("raw") || candidate.eq_ignore_ascii_case("conflicts") {
        return true;
    }
    line_range_only_re(candidate)
}

/// FILE_LINE_RANGE_ONLY_RE: `^<range list>$` (case-insensitive).
fn line_range_only_re(candidate: &str) -> bool {
    let lower = candidate.to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    // Chunk grammar: L?\d+(?:(?:[-+]|\.\.)L?\d+|-|\.\.)?
    candidate.split(',').all(range_chunk_re)
}

fn range_chunk_re(chunk: &str) -> bool {
    let mut rest = chunk;
    if rest.starts_with(['L', 'l']) {
        rest = &rest[1..];
    }
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    rest = &rest[digits.len()..];
    if rest.is_empty() {
        return true;
    }
    if let Some(after) = rest.strip_prefix("..") {
        rest = after;
    } else if let Some(after) = rest.strip_prefix(['-', '+']) {
        rest = after;
    } else {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    if rest.starts_with(['L', 'l']) {
        rest = &rest[1..];
    }
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// FILE_RAW_ONLY_RE: `/^raw$/i`.
fn raw_only_re(candidate: &str) -> bool {
    candidate.eq_ignore_ascii_case("raw")
}

/// `isProbablyBinary` equivalent: NUL byte or invalid UTF-8.
fn is_probably_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

pub(crate) async fn read(
    ctx: &CodingEngineContext,
    input: Value,
) -> Result<String, CodingEngineError> {
    let Some(path) = input.get("path").and_then(Value::as_str) else {
        return Err(input_error("read requires a string `path`"));
    };

    // `file://` URLs expand to local paths in the pinned tool; our engines
    // operate on scoped paths only, so leave the path verbatim.

    // `splitPathAndSelPreferringLiteral` (pinned path-utils.ts): a literal
    // filesystem entry named like `test:1-2` / `log:raw` wins over selector
    // interpretation; only a definitive miss falls back to the strict split.
    let strict = split_path_and_sel(path);
    if path.starts_with("artifact://") {
        if let Some((artifact_url, byte_range)) = path.rsplit_once(":bytes:") {
            let selector = format!("bytes:{byte_range}");
            return read_artifact(ctx, artifact_url, Some(&selector)).await;
        }
        return read_artifact(ctx, &strict.0, strict.1.as_deref()).await;
    }
    let (local_path, sel) = if strict.1.is_some() && literal_path_exists(ctx, path).await? {
        (path.to_string(), None)
    } else {
        strict
    };
    let parsed = parse_sel(sel.as_deref())
        .map_err(|message| coding_error(CodingEngineErrorKind::InvalidSelector, message))?;
    let resolved = resolve_input_path(ctx, &local_path, FilesystemOperation::ReadFile)?;
    let display = display_path(
        &workspace_virtual_root(ctx).ok_or_else(|| {
            coding_error(
                CodingEngineErrorKind::PathResolution,
                "no workspace mount root".to_string(),
            )
        })?,
        &resolved.virtual_path,
    );

    let stat = match ctx.filesystem.stat(&resolved.virtual_path).await {
        Ok(stat) => Some(stat),
        Err(FilesystemError::NotFound { .. }) if resolved.is_mount_root() => None,
        Err(FilesystemError::NotFound { .. }) => {
            return Err(coding_error(
                CodingEngineErrorKind::PathNotFound,
                format!("Path '{local_path}' not found"),
            ));
        }
        Err(error) => {
            return Err(coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            ));
        }
    };

    let is_directory = stat
        .as_ref()
        .is_some_and(|stat| stat.file_type == FileType::Directory);
    let file_size = stat.as_ref().map(|stat| stat.len).unwrap_or(0);

    if stat.as_ref().is_some_and(|stat| stat.sensitive) {
        return Err(filesystem_denied());
    }
    if !is_directory && file_size > MAX_READ_SIZE {
        return Err(read_limit_exceeded());
    }

    if is_directory {
        if parsed.is_multi_range() {
            return Err(coding_error(
                CodingEngineErrorKind::MultiRangeDirectory,
                "Multi-range line selectors are not supported for directory listings.",
            ));
        }
        let (offset, limit) = sel_to_offset_limit(&parsed);
        return read_directory(ctx, &resolved, &display, offset, limit).await;
    }

    if matches!(parsed, ParsedSelector::Conflicts) {
        return read_file_conflicts(ctx, &resolved, &display).await;
    }

    let Some(versioned) = ctx
        .filesystem
        .get(&resolved.virtual_path)
        .await
        .map_err(|error| {
            coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            )
        })?
    else {
        return Err(coding_error(
            CodingEngineErrorKind::PathNotFound,
            format!("Path '{local_path}' not found"),
        ));
    };
    let bytes = versioned.entry.body;

    if !parsed.is_raw() && is_probably_binary(&bytes) {
        return Ok(format!(
            "[Cannot read binary file '{display}' ({}); not valid UTF-8 text. Use ':raw' to read bytes verbatim.]",
            format_bytes(file_size)
        ));
    }

    let text = String::from_utf8(bytes).map_err(|_| {
        if parsed.is_raw() {
            // `:raw` was explicitly requested: the pinned tool returns raw
            // bytes, but this engine's JSON string output cannot carry
            // them. A self-contradictory "use :raw" instruction is wrong
            // here.
            coding_error(
                CodingEngineErrorKind::Filesystem,
                format!(
                    "[Cannot read binary file '{display}' ({}); binary bytes cannot be returned in JSON output.]",
                    format_bytes(file_size)
                ),
            )
        } else {
            coding_error(
                CodingEngineErrorKind::HashlineApply,
                format!(
                    "[Cannot read binary file '{display}' ({}); not valid UTF-8 text. Use ':raw' to read bytes verbatim.]",
                    format_bytes(file_size)
                ),
            )
        }
    })?;

    if parsed.is_multi_range() {
        return read_multi_range(ctx, &resolved, &display, &parsed, &text);
    }

    let (offset, limit) = sel_to_offset_limit(&parsed);
    read_single_range(ctx, &resolved, &display, &parsed, &text, offset, limit)
}

async fn read_artifact(
    ctx: &CodingEngineContext,
    artifact_url: &str,
    sel: Option<&str>,
) -> Result<String, CodingEngineError> {
    let artifact_ref = artifact_url
        .parse::<ArtifactRef>()
        .map_err(|error| coding_error(CodingEngineErrorKind::PathResolution, error.to_string()))?;
    let byte_range = sel
        .and_then(|selector| selector.strip_prefix("bytes:"))
        .map(parse_artifact_byte_range)
        .transpose()?;
    let parsed = if byte_range.is_some() {
        ParsedSelector::None
    } else {
        parse_sel(sel)
            .map_err(|message| coding_error(CodingEngineErrorKind::InvalidSelector, message))?
    };
    if matches!(parsed, ParsedSelector::Conflicts) {
        return Err(coding_error(
            CodingEngineErrorKind::InvalidSelector,
            "Conflict selectors are not supported for artifacts.".to_string(),
        ));
    }
    let ranges = match &parsed {
        ParsedSelector::Lines { ranges, .. } => {
            let mut remaining = DEFAULT_MAX_LINES;
            ranges
                .iter()
                .filter_map(|range| {
                    if remaining == 0 {
                        return None;
                    }
                    let requested_end = range.end_line.unwrap_or_else(|| {
                        range
                            .start_line
                            .saturating_add(DEFAULT_LIMIT.saturating_sub(1))
                    });
                    let end = requested_end
                        .min(range.start_line.saturating_add(remaining.saturating_sub(1)));
                    remaining = remaining
                        .saturating_sub(end.saturating_sub(range.start_line).saturating_add(1));
                    Some(ArtifactLineRange {
                        start: range.start_line,
                        end,
                    })
                })
                .collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let selector = match (byte_range, &parsed, ranges.as_slice()) {
        (Some(range), _, _) => ArtifactSelector::Bytes(range),
        (_, ParsedSelector::Raw, _) => ArtifactSelector::Full,
        (_, ParsedSelector::Lines { raw: true, .. }, [range]) => ArtifactSelector::RawLines(*range),
        (_, ParsedSelector::Lines { .. }, [range]) => ArtifactSelector::Lines(*range),
        (_, ParsedSelector::Lines { .. }, ranges) => ArtifactSelector::MultiLines(ranges.to_vec()),
        _ => ArtifactSelector::Lines(ArtifactLineRange {
            start: 1,
            end: DEFAULT_LIMIT,
        }),
    };
    let reader = ctx.artifact_reader.as_ref().ok_or_else(|| {
        coding_error(
            CodingEngineErrorKind::Filesystem,
            "No session - artifacts unavailable".to_string(),
        )
    })?;
    // Selector-aware byte budget, mirroring the pinned `#readArtifactFile`:
    // line-oriented reads budget `max(DEFAULT_MAX_BYTES, lines * 512)` so the
    // requested line count actually fits. Bare reads become a bounded
    // 3000-line range; unbounded raw reads retain the upstream 50 KiB guard.
    let max_output_bytes = match &selector {
        ArtifactSelector::Bytes(range) => range.end.saturating_sub(range.start).saturating_add(1),
        ArtifactSelector::Lines(range) | ArtifactSelector::RawLines(range) => {
            let span = range.end.saturating_sub(range.start).saturating_add(1);
            (DEFAULT_MAX_BYTES as u64).max(span * BYTES_PER_LINE_BUDGET as u64)
        }
        ArtifactSelector::MultiLines(ranges) => {
            let span = ranges
                .iter()
                .map(|range| range.end.saturating_sub(range.start).saturating_add(1))
                .sum::<u64>();
            (DEFAULT_MAX_BYTES as u64).max(span * BYTES_PER_LINE_BUDGET as u64)
        }
        ArtifactSelector::Full if matches!(parsed, ParsedSelector::Raw) => DEFAULT_MAX_BYTES as u64,
        ArtifactSelector::Full => {
            (DEFAULT_MAX_BYTES as u64).max(DEFAULT_MAX_LINES * BYTES_PER_LINE_BUDGET as u64)
        }
        _ => (DEFAULT_MAX_BYTES as u64).max(DEFAULT_MAX_LINES * BYTES_PER_LINE_BUDGET as u64),
    };
    let chunk = reader
        .read(ArtifactReadTarget {
            artifact_id: artifact_ref.id(),
            selector,
            max_output_bytes,
        })
        .await
        .map_err(|error| {
            let message = if error == ArtifactAccessError::OversizedUnsliced
                && matches!(parsed, ParsedSelector::Raw)
            {
                format!(
                    "Unbounded raw read blocked for {artifact_url}. Reading the whole artifact verbatim can exhaust memory. Use {artifact_url}:raw:1-3000 for bounded verbatim chunks or {artifact_url}:1-3000 for numbered exploration."
                )
            } else if error == ArtifactAccessError::OversizedUnsliced {
                format!(
                    "Artifact {} exceeds the read limit. Use bounded selectors such as artifact://{}:1-100, :raw:1-100, or :bytes:0-3071, then continue.",
                    artifact_ref.id().get(),
                    artifact_ref.id().get(),
                )
            } else {
                format!("Cannot read artifact: {error}")
            };
            coding_error(CodingEngineErrorKind::Filesystem, message)
        })?
        .ok_or_else(|| {
            coding_error(
                CodingEngineErrorKind::PathNotFound,
                format!("Artifact {} not found", artifact_ref.id().get()),
            )
        })?;
    let text = String::from_utf8(chunk.content).map_err(|_| {
        coding_error(
            CodingEngineErrorKind::Filesystem,
            format!(
                "[Cannot read binary artifact '{artifact_url}' ({}); binary bytes cannot be returned in JSON output.]",
                format_bytes(chunk.total_bytes)
            ),
        )
    })?;
    if byte_range.is_some() || parsed.is_raw() || text.is_empty() {
        return Ok(text);
    }

    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .collect();
    let line_numbers: Vec<u64> = match &parsed {
        ParsedSelector::Lines { ranges, .. } => ranges
            .iter()
            .flat_map(|range| {
                let end = range.end_line.unwrap_or(u64::MAX);
                range.start_line..=end
            })
            .take(DEFAULT_MAX_LINES as usize)
            .collect(),
        _ => (1..=DEFAULT_MAX_LINES).collect(),
    };
    let rendered: Vec<String> = lines
        .iter()
        .zip(line_numbers.iter())
        .map(|(line, number)| format_numbered_line(*number, line))
        .collect();
    let rendered_lines = rendered.len() as u64;
    if rendered.is_empty() {
        return Ok(String::new());
    }
    let mut output = rendered.join("\n");
    // Elision footer mirrors the pinned file-read path: when the artifact has
    // more lines than the rendered span, tell the model how to continue.
    let total_lines = chunk.total_lines.unwrap_or(0);
    let start_line = line_numbers[0];
    let end_line = line_numbers[rendered_lines as usize - 1];
    if total_lines > end_line {
        let remaining = total_lines - end_line;
        output.push_str(&format!(
            "\n\n[{remaining} more lines in artifact. Use {artifact_url}:{} to continue]",
            end_line + 1
        ));
    } else if start_line > 1 && chunk.total_lines.is_none() {
        // Reader did not report total lines; only note continuation when a
        // bounded selector was used (the model may page further).
        output.push_str(&format!(
            "\n\n[Use {artifact_url}:{} to continue]",
            end_line + 1
        ));
    }
    Ok(output)
}

fn parse_artifact_byte_range(range: &str) -> Result<ArtifactByteRange, CodingEngineError> {
    let invalid = || {
        coding_error(
            CodingEngineErrorKind::InvalidSelector,
            format!(
                "Invalid artifact byte selector ':bytes:{range}'. Use :bytes:START-END with zero-based inclusive byte offsets."
            ),
        )
    };
    let (start, end) = range.split_once('-').ok_or_else(invalid)?;
    if start.is_empty() || end.is_empty() {
        return Err(invalid());
    }
    let start = start.parse::<u64>().map_err(|_| invalid())?;
    let end = end.parse::<u64>().map_err(|_| invalid())?;
    if end < start {
        return Err(invalid());
    }
    Ok(ArtifactByteRange { start, end })
}

/// The pinned `#readDirectory`: builds the workspace tree (maxDepth 2,
/// per-dir child cap 12, root uncapped), renders it, then slices by
/// offset/limit.
async fn read_directory(
    ctx: &CodingEngineContext,
    resolved: &super::ResolvedCodingPath,
    _display: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<String, CodingEngineError> {
    const READ_DIRECTORY_MAX_DEPTH: usize = 2;
    const READ_DIRECTORY_CHILD_LIMIT: usize = 12;

    // A missing mount ROOT lists as empty (the grant names it).
    let entries = match ctx.filesystem.list_dir(&resolved.virtual_path).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) if resolved.is_mount_root() => Vec::new(),
        Err(error) => {
            return Err(coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("Cannot read directory: {error}"),
            ));
        }
    };
    let rendered = render_directory_tree(
        ctx,
        &resolved.virtual_path,
        &entries,
        READ_DIRECTORY_MAX_DEPTH,
        READ_DIRECTORY_CHILD_LIMIT,
    )
    .await?;
    let total_lines = rendered.lines().count();

    let output = if total_lines <= 1 {
        "(empty directory)".to_string()
    } else {
        rendered
    };

    let wants_slice = offset.is_some() || limit.is_some();
    if wants_slice {
        let all_lines: Vec<&str> = output.split('\n').collect();
        let start = offset.map(|offset| offset.saturating_sub(1)).unwrap_or(0) as usize;
        if start >= all_lines.len() {
            let suggestion = if all_lines.is_empty() {
                "The listing is empty.".to_string()
            } else {
                format!(
                    "Use :1 to read from the start, or :{} to read the last line.",
                    all_lines.len()
                )
            };
            return Ok(format!(
                "Line {} is beyond end of listing ({} lines total). {suggestion}",
                start + 1,
                all_lines.len()
            ));
        }
        let end = limit
            .map(|limit| {
                (start as u64)
                    .saturating_add(limit)
                    .min(all_lines.len() as u64) as usize
            })
            .unwrap_or(all_lines.len());
        let mut text = all_lines[start..end].join("\n");
        if end < all_lines.len() {
            let remaining = all_lines.len() - end;
            text.push_str(&format!(
                "\n\n[{remaining} more lines in listing. Use :{} to continue]",
                end + 1
            ));
        }
        return Ok(text);
    }

    Ok(output)
}

/// Assemble + render the directory tree (`assembleTree` + `renderNode` +
/// `formatLines` from the pinned `workspace-tree.ts`; the native walker's
/// non-source-dir pruning and recency sort are mirrored here).
async fn render_directory_tree(
    ctx: &CodingEngineContext,
    root: &ironclaw_host_api::path::VirtualPath,
    entries: &[ironclaw_filesystem::DirEntry],
    max_depth: usize,
    per_dir_limit: usize,
) -> Result<String, CodingEngineError> {
    const EXCLUDED_DIRS: &[&str] = &[
        "node_modules",
        ".git",
        ".next",
        "dist",
        "build",
        "target",
        ".venv",
        ".cache",
        ".turbo",
        ".parcel-cache",
        "coverage",
    ];
    #[derive(Debug, Clone)]
    struct Node {
        name: String,
        is_dir: bool,
        mtime_ms: u64,
        size: u64,
        depth: usize,
        children: Vec<Node>,
        dropped_count: usize,
    }

    // The pinned `buildDirectoryTree` receives a single native recursive
    // scan (`listWorkspace({ maxDepth })`); our backend only lists one
    // level per call, so recurse into subdirectories here to mirror it.
    // Each collected entry carries the successful stat from the walk so the
    // bucketing pass reuses `modified`/`len` instead of stat'ing again.
    let mut all_entries: Vec<(ironclaw_filesystem::DirEntry, ironclaw_filesystem::FileStat)> =
        Vec::new();
    let mut frontier: Vec<(usize, ironclaw_filesystem::DirEntry)> =
        entries.iter().cloned().map(|entry| (1, entry)).collect();
    let mut visited = 0usize;
    while let Some((depth, entry)) = frontier.pop() {
        visited = visited.saturating_add(1);
        if visited > MAX_VISITED_ENTRIES {
            return Err(coding_error(
                CodingEngineErrorKind::ResourceLimit,
                "workspace traversal exceeds the entry limit",
            ));
        }
        if entry.file_type == FileType::Directory && EXCLUDED_DIRS.contains(&entry.name.as_str()) {
            continue;
        }
        let stat = match ctx.filesystem.stat(&entry.path).await {
            Ok(stat) => stat,
            Err(FilesystemError::NotFound { .. }) => continue,
            Err(error) => {
                tracing::debug!(path = entry.path.as_str(), %error, "skipping directory entry after stat failed");
                continue;
            }
        };
        if stat.sensitive {
            continue;
        }
        all_entries.push((entry.clone(), stat));
        if entry.file_type != FileType::Directory || depth >= max_depth {
            continue;
        }
        match ctx.filesystem.list_dir(&entry.path).await {
            Ok(children) => frontier.extend(
                children
                    .into_iter()
                    .map(|child| (depth.saturating_add(1), child)),
            ),
            Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => {
                return Err(coding_error(
                    CodingEngineErrorKind::Filesystem,
                    format!("filesystem error: {error}"),
                ));
            }
        }
    }

    // Bucket entries by parent path.
    let mut by_parent: std::collections::BTreeMap<String, Vec<Node>> =
        std::collections::BTreeMap::new();
    let root_str = root.as_str();
    for (entry, stat) in &all_entries {
        let raw = entry.path.as_str();
        let Some(relative) = raw
            .strip_prefix(root_str)
            .map(|tail| tail.trim_start_matches('/'))
        else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let segments: Vec<&str> = relative.split('/').collect();
        if segments.len() > max_depth {
            continue;
        }
        let name = segments.last().copied().unwrap_or_default().to_string();
        let parent = segments[..segments.len() - 1].join("/");
        // The stat was already fetched (and sensitivity-checked) during the
        // frontier walk; reuse it instead of a second filesystem round-trip.
        let mtime_ms = stat
            .modified
            .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let node = Node {
            name,
            is_dir: entry.file_type == FileType::Directory,
            mtime_ms,
            size: stat.len,
            depth: segments.len(),
            children: Vec::new(),
            dropped_count: 0,
        };
        by_parent.entry(parent).or_default().push(node);
    }

    let mut root_node = Node {
        name: ".".to_string(),
        is_dir: true,
        mtime_ms: 0,
        size: 0,
        depth: 0,
        children: Vec::new(),
        dropped_count: 0,
    };

    fn sort_by_recency(mut nodes: Vec<Node>) -> Vec<Node> {
        nodes.sort_by(|a, b| {
            b.mtime_ms
                .cmp(&a.mtime_ms)
                .then_with(|| a.name.cmp(&b.name))
        });
        nodes
    }

    /// Cap `all` to `limit` children (`[recent…, oldest]`) and return the
    /// number of dropped children for the parent's `… N more` marker.
    fn cap_children(all: &mut Vec<Node>, limit: usize) -> usize {
        let len = all.len();
        if len <= limit {
            return 0;
        }
        let dropped = len - limit;
        // `len > limit` (checked above) with the caller's constant limit
        // (>= 1) implies the vector is non-empty, so `pop` is guaranteed
        // Some; fail closed by leaving the list uncapped rather than
        // panicking.
        let Some(oldest) = all.pop() else {
            return 0;
        };
        all.truncate(limit - 1);
        all.push(oldest);
        dropped
    }

    fn find_node_mut<'a>(node: &'a mut Node, rel_path: &str) -> Option<&'a mut Node> {
        if rel_path.is_empty() {
            return Some(node);
        }
        let (head, tail) = rel_path.split_once('/').unwrap_or((rel_path, ""));
        let child = node.children.iter_mut().find(|child| child.name == head)?;
        if tail.is_empty() {
            Some(child)
        } else {
            find_node_mut(child, tail)
        }
    }

    // Depth-first assembly: root is UNCAPPED (`rootLimit: null` in the
    // pinned read tool); deeper directories cap at `per_dir_limit` with the
    // "… N more" marker, keeping the oldest entry visible.
    let mut stack: Vec<(usize, String)> = Vec::new();
    {
        let all = by_parent.get("").cloned().unwrap_or_default();
        let all = sort_by_recency(all);
        root_node.children = all;
        for child in &root_node.children {
            if child.is_dir {
                stack.push((1, child.name.clone()));
            }
        }
    }
    while let Some((depth, rel_path)) = stack.pop() {
        if depth >= max_depth {
            continue;
        }
        let all = by_parent.get(&rel_path).cloned().unwrap_or_default();
        let mut all = sort_by_recency(all);
        let dropped = cap_children(&mut all, per_dir_limit);
        let target = find_node_mut(&mut root_node, &rel_path);
        if let Some(target) = target {
            target.dropped_count = dropped;
            target.children = all;
            for child in &target.children {
                if child.is_dir {
                    let child_rel = if rel_path.is_empty() {
                        child.name.clone()
                    } else {
                        format!("{rel_path}/{}", child.name)
                    };
                    stack.push((depth + 1, child_rel));
                }
            }
        }
    }

    // Render (renderNode + formatLines).
    #[derive(Clone)]
    struct RenderedLine {
        label: String,
        size: Option<String>,
        age: Option<String>,
    }

    let now = SystemTime::now();
    fn render_node(node: &Node, out: &mut Vec<RenderedLine>, now: SystemTime) {
        if node.depth == 0 {
            out.push(RenderedLine {
                label: node.name.clone(),
                size: None,
                age: None,
            });
        } else {
            let indent = "  ".repeat(node.depth);
            let suffix = if node.is_dir { "/" } else { "" };
            let age = if node.mtime_ms == 0 {
                None
            } else {
                Some(format_age(
                    now.duration_since(SystemTime::UNIX_EPOCH)
                        .ok()
                        .map(|duration| duration.as_millis() as u64)
                        .unwrap_or(0)
                        .saturating_sub(node.mtime_ms)
                        / 1000,
                ))
            };
            out.push(RenderedLine {
                label: format!("{indent}- {}{suffix}", node.name),
                size: if node.is_dir {
                    None
                } else {
                    Some(format_bytes(node.size))
                },
                age,
            });
        }
        if node.dropped_count == 0 {
            for child in &node.children {
                render_node(child, out, now);
            }
            return;
        }
        // Layout: recent children, then "… N more" marker, then oldest.
        let (recent, oldest) = node.children.split_at(node.children.len() - 1);
        for child in recent {
            render_node(child, out, now);
        }
        let child_depth = node.depth + 1;
        out.push(RenderedLine {
            label: format!(
                "{}- … {} more",
                "  ".repeat(child_depth),
                node.dropped_count
            ),
            size: None,
            age: None,
        });
        if let Some(oldest) = oldest.first() {
            render_node(oldest, out, now);
        }
    }

    let mut raw_lines: Vec<RenderedLine> = Vec::new();
    render_node(&root_node, &mut raw_lines, now);

    let max_label_length = raw_lines
        .iter()
        .map(|line| line.label.len())
        .max()
        .unwrap_or(0);
    let rendered = raw_lines
        .iter()
        .map(|line| {
            if line.age.is_none() {
                return line.label.clone();
            }
            let size_column = line.size.clone().unwrap_or_default();
            let padded_size = format!("{size_column:<8}");
            let age = line.age.clone().unwrap_or_default();
            format!(
                "{:<width$}  {padded_size}  {:<4}",
                line.label,
                age,
                width = max_label_length + 2
            )
            .trim_end()
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(rendered)
}

/// `#readFileConflicts`: scan the file once and return a compact summary.
async fn read_file_conflicts(
    ctx: &CodingEngineContext,
    resolved: &super::ResolvedCodingPath,
    display: &str,
) -> Result<String, CodingEngineError> {
    let text = read_text(ctx, resolved).await?;
    let blocks = scan_conflict_lines(&text, 1);
    if blocks.is_empty() {
        return Ok(format!("No unresolved git merge conflicts in {display}."));
    }
    let entries: Vec<ConflictEntry> = blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| ConflictEntry {
            id: index + 1,
            start_line: block.start_line,
            end_line: block.end_line,
            ours_label: block.ours_label.clone(),
            base_label: block.base_label.clone(),
            theirs_label: block.theirs_label.clone(),
            has_base: block.base_lines.is_some(),
        })
        .collect();
    Ok(format_conflict_summary(&entries, display))
}

async fn read_text(
    ctx: &CodingEngineContext,
    resolved: &super::ResolvedCodingPath,
) -> Result<String, CodingEngineError> {
    let Some(versioned) = ctx
        .filesystem
        .get(&resolved.virtual_path)
        .await
        .map_err(|error| {
            coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            )
        })?
    else {
        return Err(coding_error(
            CodingEngineErrorKind::PathNotFound,
            format!("Path '{}' not found", resolved.virtual_path.as_str()),
        ));
    };
    String::from_utf8(versioned.entry.body).map_err(|_| {
        coding_error(
            CodingEngineErrorKind::Filesystem,
            "file is not valid UTF-8 text".to_string(),
        )
    })
}

#[derive(Debug, Clone)]
struct ConflictBlock {
    start_line: u64,
    end_line: u64,
    ours_label: Option<String>,
    base_label: Option<String>,
    theirs_label: Option<String>,
    base_lines: Option<Vec<String>>,
}

/// `scanConflictLines` from the pinned `conflict-detect.ts`.
fn scan_conflict_lines(lines: &str, first_line_number: u64) -> Vec<ConflictBlock> {
    const OURS_PREFIX: &str = "<<<<<<<";
    const BASE_PREFIX: &str = "|||||||";
    const SEPARATOR: &str = "=======";
    const THEIRS_PREFIX: &str = ">>>>>>>";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Idle,
        Ours,
        Base,
        Theirs,
    }

    fn match_marker(line: &str, prefix: &str) -> Option<String> {
        line.strip_prefix(prefix).map(|rest| {
            let label = rest.trim();
            if label.is_empty() {
                String::new()
            } else {
                label.to_string()
            }
        })
    }

    #[derive(Debug)]
    struct Partial {
        start_line: u64,
        ours_label: Option<String>,
        base_label: Option<String>,
        base_lines: Option<Vec<String>>,
        separator_line: Option<u64>,
        theirs_lines: Option<Vec<String>>,
    }

    let mut blocks: Vec<ConflictBlock> = Vec::new();
    let mut phase = Phase::Idle;
    let mut partial: Option<Partial> = None;

    for (index, raw_line) in lines.split('\n').enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let ln = first_line_number + index as u64;

        if let Some(label) = match_marker(line, OURS_PREFIX) {
            partial = Some(Partial {
                start_line: ln,
                ours_label: if label.is_empty() { None } else { Some(label) },
                base_label: None,
                base_lines: None,
                separator_line: None,
                theirs_lines: None,
            });
            phase = Phase::Ours;
            continue;
        }

        if phase == Phase::Idle || partial.is_none() {
            continue;
        }

        // Every marker below that sets a phase also sets `partial`, and the
        // two are cleared together, so `partial` is Some whenever the guard
        // above passes. Bind it once instead of re-checking per use; the
        // reset paths below run only after `p`'s last use on their path.
        let p = match partial.as_mut() {
            Some(p) => p,
            None => continue, // unreachable: guarded above
        };

        if let Some(label) = match_marker(line, BASE_PREFIX) {
            if phase != Phase::Ours {
                partial = None;
                phase = Phase::Idle;
                continue;
            }
            p.base_label = if label.is_empty() { None } else { Some(label) };
            p.base_lines = Some(Vec::new());
            phase = Phase::Base;
            continue;
        }

        if line == SEPARATOR {
            if phase == Phase::Ours || phase == Phase::Base {
                p.separator_line = Some(ln);
                p.theirs_lines = Some(Vec::new());
                phase = Phase::Theirs;
            } else {
                partial = None;
                phase = Phase::Idle;
            }
            continue;
        }

        if let Some(label) = match_marker(line, THEIRS_PREFIX) {
            if phase == Phase::Theirs && p.separator_line.is_some() && p.theirs_lines.is_some() {
                blocks.push(ConflictBlock {
                    start_line: p.start_line,
                    end_line: ln,
                    ours_label: p.ours_label.clone(),
                    base_label: p.base_label.clone(),
                    theirs_label: if label.is_empty() { None } else { Some(label) },
                    base_lines: p.base_lines.clone(),
                });
            }
            partial = None;
            phase = Phase::Idle;
            continue;
        }

        match phase {
            Phase::Ours => {}
            Phase::Base => {
                if let Some(base_lines) = p.base_lines.as_mut() {
                    base_lines.push(line.to_string());
                }
            }
            Phase::Theirs => {
                if let Some(theirs_lines) = p.theirs_lines.as_mut() {
                    theirs_lines.push(line.to_string());
                }
            }
            Phase::Idle => {}
        }
    }

    blocks
}

struct ConflictEntry {
    id: usize,
    start_line: u64,
    end_line: u64,
    ours_label: Option<String>,
    base_label: Option<String>,
    theirs_label: Option<String>,
    has_base: bool,
}

/// `formatConflictSummary` from the pinned `conflict-detect.ts` (with the
/// conflict-registry NOTICE block; `scanTruncated` is always false here).
fn format_conflict_summary(entries: &[ConflictEntry], display_path: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let total = entries.len();
    let word = if total == 1 { "conflict" } else { "conflicts" };
    lines.push(format!("⚠ {total} unresolved {word} in {display_path}"));
    let ours_label = pick_label(entries, |entry| entry.ours_label.clone());
    let theirs_label = pick_label(entries, |entry| entry.theirs_label.clone());
    let base_label = pick_label(entries, |entry| entry.base_label.clone());
    let any_base = entries.iter().any(|entry| entry.has_base);
    if let Some(ours) = ours_label {
        lines.push(format!("- ours = {ours}"));
    }
    if let Some(theirs) = theirs_label {
        lines.push(format!("- theirs = {theirs}"));
    }
    if any_base {
        lines.push(format!(
            "- base = {}",
            base_label.unwrap_or_else(|| "(no label)".to_string())
        ));
    }
    lines.push(
        "NOTICE: Bulk-resolve with `write({ path: \"conflict://*\", content })`, or address a single block with `write({ path: \"conflict://<N>\", content })`. Inspect a block by reading `conflict://<N>` (add `/ours` / `/theirs` / `/base` for a single side).".to_string(),
    );
    lines.push(
        "`content` shorthand: `@ours` / `@theirs` / `@base` / `@both` lines expand to the recorded sections; `@both` = ours-then-theirs (additive conflicts only — never for competing edits of the same lines). Per-id bulk: content of `<id>: @side` lines (e.g. \"1: @ours\\n2: @theirs\") resolves each listed id in one call. Non-token lines pass through verbatim. Writes replace ONLY the marker block — never repeat the surrounding lines. Keep one side or combine faithfully; never invent content beyond the recorded sides.".to_string(),
    );
    lines.push(String::new());
    let id_width = entries
        .last()
        .map(|entry| entry.id.to_string().len())
        .unwrap_or(1);
    for entry in entries {
        let range = if entry.start_line == entry.end_line {
            format!("L{}", entry.start_line)
        } else {
            format!("L{}-{}", entry.start_line, entry.end_line)
        };
        let id_cell = format!("#{:<width$}", entry.id, width = id_width);
        let kind = if entry.has_base { "  (3-way)" } else { "" };
        lines.push(format!("{id_cell}  {range}{kind}"));
    }
    lines.join("\n")
}

fn pick_label(
    entries: &[ConflictEntry],
    get: impl Fn(&ConflictEntry) -> Option<String>,
) -> Option<String> {
    for entry in entries {
        let label = get(entry)?;
        if !label.trim().is_empty() {
            return Some(label);
        }
    }
    None
}

/// Single-range (or whole-file) read: numbered rows with the hashline
/// header, leading/trailing context, and the pinned truncation notices.
fn read_single_range(
    ctx: &CodingEngineContext,
    resolved: &super::ResolvedCodingPath,
    display: &str,
    parsed: &ParsedSelector,
    text: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<String, CodingEngineError> {
    let raw_selector = parsed.is_raw();
    let all_lines: Vec<&str> = text.split('\n').collect();
    let total_lines = all_lines.len() as u64;

    let requested_start = offset.map(|offset| offset.saturating_sub(1)).unwrap_or(0);
    let expand_start = !raw_selector && offset.is_some_and(|offset| offset > 1);
    let expand_end = !raw_selector && limit.is_some();
    let leading_context = if expand_start {
        requested_start.min(1)
    } else {
        0
    };
    let trailing_context = if expand_end { 3 } else { 0 };
    let start_line = requested_start - leading_context;
    let start_line_display = start_line + 1;

    let effective_limit = limit.unwrap_or(DEFAULT_LIMIT);
    let max_lines_to_collect = effective_limit
        .saturating_add(leading_context)
        .saturating_add(trailing_context)
        .min(DEFAULT_MAX_LINES);
    let max_bytes_for_read = (DEFAULT_MAX_BYTES as u64)
        .max(max_lines_to_collect * BYTES_PER_LINE_BUDGET as u64)
        as usize;

    if requested_start >= total_lines {
        let suggestion = if total_lines == 0 {
            "The file is empty.".to_string()
        } else {
            format!("Use :1 to read from the start, or :{total_lines} to read the last line.")
        };
        return Ok(format!(
            "Line {} is beyond end of file ({total_lines} lines total). {suggestion}",
            requested_start + 1
        ));
    }

    // Collect lines within the byte budget (never partial lines).
    let mut collected_lines: Vec<&str> = Vec::new();
    let mut collected_bytes = 0usize;
    let mut stopped_by_byte_limit = false;
    let mut first_line_byte_length: Option<usize> = None;
    let mut first_line_preview: Option<String> = None;
    let end_line = (start_line + max_lines_to_collect).min(total_lines);
    for line_index in start_line..end_line {
        let line = all_lines.get(line_index as usize).copied().unwrap_or("");
        let sep_bytes = if collected_lines.is_empty() { 0 } else { 1 };
        let line_bytes = line.len();
        if line_bytes + sep_bytes + collected_bytes > max_bytes_for_read {
            stopped_by_byte_limit = true;
            if collected_lines.is_empty() {
                first_line_byte_length = Some(line_bytes);
                first_line_preview = Some(if line_bytes > DEFAULT_MAX_BYTES {
                    // truncateHeadBytes: keep the first maxBytes chars/bytes.
                    let cut = line.floor_char_boundary(DEFAULT_MAX_BYTES);
                    line[..cut].to_string()
                } else {
                    line.to_string()
                });
            }
            break;
        }
        collected_bytes += sep_bytes + line_bytes;
        collected_lines.push(line);
    }
    let reached_eof = true; // we always have the whole file in memory
    let total_selected_lines = total_lines - start_line;
    let was_truncated =
        (collected_lines.len() as u64) < total_selected_lines || stopped_by_byte_limit;
    let output_lines = collected_lines.len() as u64;
    let next_offset = start_line_display + output_lines;

    let should_add_hash_lines = !raw_selector;
    let selected_content = collected_lines.join("\n");
    let formatted_body = if raw_selector {
        selected_content
    } else {
        format_numbered_lines(&selected_content, start_line_display)
    };

    let mut output_text: String;
    let mut truncation_notice: Option<String> = None;

    if first_line_byte_length.is_some() {
        let first_line_bytes = first_line_byte_length.unwrap_or(0);
        let snippet = first_line_preview.clone().unwrap_or_default();
        if should_add_hash_lines {
            output_text = format!(
                "[Line {start_line_display} is {}, exceeds {} limit. Hashline output requires full lines; cannot emit an editable numbered preview for a truncated line.]",
                format_bytes(first_line_bytes as u64),
                format_bytes(max_bytes_for_read as u64)
            );
        } else {
            output_text = snippet.clone();
        }
        if snippet.is_empty() {
            output_text = format!(
                "[Line {start_line_display} is {}, exceeds {} limit. Unable to display a valid UTF-8 snippet.]",
                format_bytes(first_line_bytes as u64),
                format_bytes(max_bytes_for_read as u64)
            );
        }
        let end_line_display = (start_line_display as i64 + output_lines as i64 - 1).max(0) as u64;
        truncation_notice = Some(format!(
            "[Showing lines {start_line_display}-{end_line_display} of {total_lines}. Use :{} to continue]",
            next_offset
        ));
    } else if was_truncated {
        output_text = formatted_body;
        let end_line_display = (start_line_display as i64 + output_lines as i64 - 1).max(0) as u64;
        truncation_notice = Some(format!(
            "[Showing lines {start_line_display}-{end_line_display} of {total_lines}. Use :{} to continue]",
            next_offset
        ));
    } else if start_line + output_lines < total_lines || !reached_eof {
        let remaining = total_lines - (start_line + output_lines);
        output_text = formatted_body;
        output_text.push_str(&format!(
            "\n\n[{remaining} more lines in file. Use :{next_offset} to continue]"
        ));
    } else {
        output_text = formatted_body;
    }

    // Hashline header with the whole-file tag (only when we can emit an
    // editable numbered preview and lines were collected).
    if should_add_hash_lines && !collected_lines.is_empty() && first_line_byte_length.is_none() {
        let normalized = super::hashline::normalize_to_lf(text);
        let tag = ctx.snapshots.record_and_return(
            &CodingScopeKey::from_scope(&ctx.scope, ctx.run_id),
            resolved.virtual_path.as_str(),
            &normalized,
        );
        let basename = display.rsplit('/').next().unwrap_or(display);
        output_text = format!("{}\n{output_text}", format_hashline_header(basename, &tag));
    }

    if let Some(notice) = truncation_notice {
        output_text.push_str("\n\n");
        output_text.push_str(&notice);
    }

    Ok(output_text)
}

/// Multi-range read: numbered blocks joined with `…` separators, out-of-
/// bounds notices, and the pinned elision footer when lines are elided.
fn read_multi_range(
    ctx: &CodingEngineContext,
    resolved: &super::ResolvedCodingPath,
    display: &str,
    parsed: &ParsedSelector,
    text: &str,
) -> Result<String, CodingEngineError> {
    let ParsedSelector::Lines { ranges, .. } = parsed else {
        // Unreachable: the caller dispatches here only when
        // `parsed.is_multi_range()` is true, which requires the Lines
        // variant with multiple ranges. Fail closed with an input error
        // rather than panicking.
        return Err(input_error(
            "Multi-range selectors must be a line range list.",
        ));
    };
    let raw_selector = parsed.is_raw();
    let all_lines: Vec<&str> = text.split('\n').collect();
    let total_lines = all_lines.len() as u64;

    let mut out_of_bounds: Vec<&super::selector::LineRange> = Vec::new();
    let mut visible_spans: Vec<(u64, u64)> = Vec::new();
    let mut raw_parts: Vec<String> = Vec::new();
    for range in ranges {
        if range.start_line > total_lines {
            out_of_bounds.push(range);
            continue;
        }
        let effective_end = range.end_line.unwrap_or(total_lines).min(total_lines);
        visible_spans.push((range.start_line, effective_end));
        if raw_selector {
            raw_parts.push(
                all_lines[(range.start_line - 1) as usize..effective_end as usize].join("\n"),
            );
        }
    }

    let mut output_text = String::new();
    if raw_selector {
        output_text = raw_parts.join("\n\n…\n\n");
    } else if !visible_spans.is_empty() {
        // Numbered blocks with `…` separators (and the lexical bracket
        // context rows ported from buildLineEntriesWithBlockContext).
        let entries = build_line_entries_with_context(&all_lines, &visible_spans);
        let mut lines: Vec<String> = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                LineEntry::Line {
                    line_number, text, ..
                } => {
                    lines.push(format_numbered_line(line_number, &text));
                }
                LineEntry::Ellipsis => lines.push("…".to_string()),
            }
        }
        output_text = lines.join("\n");
    }

    // Out-of-bounds notices.
    let notices: Vec<String> = out_of_bounds
        .iter()
        .map(|range| {
            let bound = match range.end_line {
                Some(end) => format!("{}-{}", range.start_line, end),
                None => format!("{}", range.start_line),
            };
            format!("[Range {bound} is beyond end of file ({total_lines} lines total); skipped]")
        })
        .collect();
    if !notices.is_empty() {
        if !output_text.is_empty() {
            output_text.push('\n');
        }
        output_text.push_str(&notices.join("\n"));
    }

    // Hashline header with the whole-file tag.
    if !raw_selector && !output_text.is_empty() {
        let normalized = super::hashline::normalize_to_lf(text);
        let tag = ctx.snapshots.record_and_return(
            &CodingScopeKey::from_scope(&ctx.scope, ctx.run_id),
            resolved.virtual_path.as_str(),
            &normalized,
        );
        let basename = display.rsplit('/').next().unwrap_or(display);
        output_text = format!("{}\n{output_text}", format_hashline_header(basename, &tag));
    }

    // Pinned elision footer (see module docs for the deviation note).
    if !raw_selector && !visible_spans.is_empty() {
        let visible_count: u64 = visible_spans
            .iter()
            .map(|(start, end)| end - start + 1)
            .sum();
        let elided_lines = total_lines.saturating_sub(visible_count);
        if elided_lines > 0 {
            let elided_ranges = elided_spans(&visible_spans, total_lines);
            let footer = format_elision_footer(display, &elided_ranges, elided_lines);
            if !footer.is_empty() {
                output_text.push_str("\n\n");
                output_text.push_str(&footer);
            }
        }
    }

    Ok(output_text)
}

/// Elided gaps between/around the visible spans, as inclusive ranges.
fn elided_spans(visible_spans: &[(u64, u64)], total_lines: u64) -> Vec<(u64, u64)> {
    let mut spans: Vec<(u64, u64)> = Vec::new();
    let mut cursor = 1u64;
    for (start, end) in visible_spans {
        if *start > cursor {
            spans.push((cursor, start - 1));
        }
        cursor = end + 1;
    }
    if cursor <= total_lines {
        spans.push((cursor, total_lines));
    }
    spans
}

/// `formatSummaryElisionFooter` from the pinned `read-format.ts`
/// (FOOTER_RANGE_SAMPLES = 2).
fn format_elision_footer(
    read_path: &str,
    elided_ranges: &[(u64, u64)],
    elided_lines: u64,
) -> String {
    if elided_ranges.is_empty() {
        return String::new();
    }
    let sample_count = elided_ranges.len().min(2);
    let selector = elided_ranges[..sample_count]
        .iter()
        .map(|(start, end)| format!("{start}-{end}"))
        .collect::<Vec<_>>()
        .join(",");
    let example = format!("{read_path}:{selector}");
    let tail = if elided_ranges.len() > sample_count {
        format!(", e.g. {example}")
    } else {
        format!(" with {example}")
    };
    format!("[…{elided_lines}ln elided; re-read needed ranges{tail}]")
}

enum LineEntry {
    Line { line_number: u64, text: String },
    Ellipsis,
}

/// `buildLineEntriesWithBlockContext` with the lexical bracket fallback
/// (the tree-sitter native path is a later-slice dependency).
fn build_line_entries_with_context(
    all_lines: &[&str],
    visible_spans: &[(u64, u64)],
) -> Vec<LineEntry> {
    let mut visible: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for (start, end) in visible_spans {
        for line in *start..=*end {
            visible.insert(line);
        }
    }
    let context = lexical_bracket_context(all_lines, &visible);
    let mut all: std::collections::BTreeSet<u64> = visible.clone();
    for line in context.keys() {
        all.insert(*line);
    }

    let mut entries: Vec<LineEntry> = Vec::new();
    let mut previous_line: Option<u64> = None;
    for line_number in all {
        if let Some(previous) = previous_line
            && line_number > previous + 1
        {
            entries.push(LineEntry::Ellipsis);
        }
        let text = all_lines
            .get((line_number - 1) as usize)
            .copied()
            .unwrap_or("")
            .to_string();
        entries.push(LineEntry::Line { line_number, text });
        previous_line = Some(line_number);
    }
    entries
}

/// `lexicalBracketContext` from the pinned `block-context.ts`: off-window
/// boundary lines (openers/closers) whose counterpart is visible.
fn lexical_bracket_context(
    all_lines: &[&str],
    visible: &std::collections::BTreeSet<u64>,
) -> std::collections::BTreeMap<u64, String> {
    const OPEN_TO_CLOSE: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    const CLOSE_TO_OPEN: [(char, char); 3] = [(')', '('), (']', '['), ('}', '{')];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Code,
        Single,
        Double,
        Template,
        BlockComment,
    }

    struct StackEntry {
        opener: char,
        line_number: u64,
        text: String,
        visible: bool,
    }

    fn is_hash_comment_start(line: &str, index: usize) -> bool {
        if !line[index..].starts_with('#') {
            return false;
        }
        line[..index].chars().all(|c| c == ' ' || c == '\t')
    }

    fn find_matching_stack_index(stack: &[StackEntry], opener: char) -> Option<usize> {
        stack.iter().rposition(|entry| entry.opener == opener)
    }

    let mut context: std::collections::BTreeMap<u64, String> = std::collections::BTreeMap::new();
    let mut stack: Vec<StackEntry> = Vec::new();
    let mut mode = Mode::Code;
    let mut escaped = false;

    for (line_index, line) in all_lines.iter().enumerate() {
        let line_number = line_index as u64 + 1;
        let line_visible = visible.contains(&line_number);
        let mut index = 0usize;
        while index < line.len() {
            // `index` advances only by char-boundary widths and stays below
            // `line.len()`, so the slice is always non-empty; skip the rest
            // of the line instead of panicking on the impossible case.
            let Some(ch) = line[index..].chars().next() else {
                break;
            };
            let next = line[index + ch.len_utf8()..].chars().next();

            if mode == Mode::BlockComment {
                if ch == '*' && next == Some('/') {
                    mode = Mode::Code;
                    index += 2;
                    continue;
                }
                index += ch.len_utf8();
                continue;
            }
            if matches!(mode, Mode::Single | Mode::Double | Mode::Template) {
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
                    Mode::Single => '\'',
                    Mode::Double => '"',
                    _ => '`',
                };
                if ch == closing {
                    mode = Mode::Code;
                }
                index += ch.len_utf8();
                continue;
            }
            if ch == '/' && next == Some('/') {
                break;
            }
            if ch == '/' && next == Some('*') {
                mode = Mode::BlockComment;
                index += 2;
                continue;
            }
            if is_hash_comment_start(line, index) {
                break;
            }
            if ch == '\'' {
                mode = Mode::Single;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '"' {
                mode = Mode::Double;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if ch == '`' {
                mode = Mode::Template;
                escaped = false;
                index += ch.len_utf8();
                continue;
            }
            if let Some((_, close)) = OPEN_TO_CLOSE.iter().find(|(open, _)| *open == ch) {
                let _ = close;
                stack.push(StackEntry {
                    opener: ch,
                    line_number,
                    text: (*line).to_string(),
                    visible: line_visible,
                });
                index += ch.len_utf8();
                continue;
            }
            if let Some((_, open)) = CLOSE_TO_OPEN.iter().find(|(close, _)| *close == ch)
                && let Some(match_index) = find_matching_stack_index(&stack, *open)
            {
                let matched = stack.remove(match_index);
                if line_visible && !matched.visible {
                    context.insert(matched.line_number, matched.text);
                }
                if matched.visible && !line_visible {
                    context.insert(line_number, (*line).to_string());
                }
            }
            index += ch.len_utf8();
        }
        if matches!(mode, Mode::Single | Mode::Double) {
            mode = Mode::Code;
            escaped = false;
        }
    }

    for line in visible {
        context.remove(line);
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_and_sel_peels_selectors() {
        assert_eq!(
            split_path_and_sel("src/foo.ts"),
            ("src/foo.ts".to_string(), None)
        );
        assert_eq!(
            split_path_and_sel("src/foo.ts:50-100"),
            ("src/foo.ts".to_string(), Some("50-100".to_string()))
        );
        assert_eq!(
            split_path_and_sel("src/foo.ts:50+10"),
            ("src/foo.ts".to_string(), Some("50+10".to_string()))
        );
        assert_eq!(
            split_path_and_sel("src/foo.ts:raw"),
            ("src/foo.ts".to_string(), Some("raw".to_string()))
        );
        assert_eq!(
            split_path_and_sel("src/foo.ts:conflicts"),
            ("src/foo.ts".to_string(), Some("conflicts".to_string()))
        );
        // Compound selectors are joined so parse_sel sees the full shape.
        assert_eq!(
            split_path_and_sel("src/foo.ts:50-100:raw"),
            ("src/foo.ts".to_string(), Some("50-100:raw".to_string()))
        );
        assert_eq!(
            split_path_and_sel("src/foo.ts:raw:50-100"),
            ("src/foo.ts".to_string(), Some("raw:50-100".to_string()))
        );
        // The strict splitter peels selector-shaped tails even when a
        // literal colon filename could exist; `read` prefers the literal
        // path via `literal_path_exists` before this split is used.
        assert_eq!(
            split_path_and_sel("a:1-2"),
            ("a".to_string(), Some("1-2".to_string()))
        );
        assert_eq!(
            split_path_and_sel("log:raw"),
            ("log".to_string(), Some("raw".to_string()))
        );
        // A literal colon filename with a non-selector tail stays intact.
        assert_eq!(split_path_and_sel("a:b"), ("a:b".to_string(), None));
    }

    #[test]
    fn format_age_matches_pinned() {
        assert_eq!(format_age(0), "");
        assert_eq!(format_age(5), "just now");
        assert_eq!(format_age(60), "1m ago");
        assert_eq!(format_age(3600), "1h ago");
        assert_eq!(format_age(86400), "1d ago");
        assert_eq!(format_age(7 * 86400), "1w ago");
        assert_eq!(format_age(30 * 86400), "1mo ago");
    }

    #[test]
    fn format_bytes_matches_pinned() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(2048), "2.0KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0GB");
    }

    #[test]
    fn elision_footer_matches_golden() {
        let ranges = vec![(50, 200)];
        assert_eq!(
            format_elision_footer("src/foo.ts", &ranges, 1200),
            "[…1200ln elided; re-read needed ranges with src/foo.ts:50-200]"
        );
        // More than two elided ranges switch to the ", e.g." tail.
        let ranges = vec![(1, 5), (10, 20), (30, 40)];
        assert_eq!(
            format_elision_footer("src/foo.ts", &ranges, 100),
            "[…100ln elided; re-read needed ranges, e.g. src/foo.ts:1-5,10-20]"
        );
    }

    #[test]
    fn conflict_scan_finds_blocks() {
        let text = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n";
        let blocks = scan_conflict_lines(text, 1);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_line, 2);
        assert_eq!(blocks[0].end_line, 6);
        assert_eq!(blocks[0].ours_label.as_deref(), Some("HEAD"));
        assert_eq!(blocks[0].theirs_label.as_deref(), Some("branch"));
        assert!(blocks[0].base_lines.is_none());
    }

    #[test]
    fn conflict_summary_empty_message() {
        let entries: Vec<ConflictEntry> = Vec::new();
        let _ = entries;
        // No blocks -> the read engine emits the empty message; the summary
        // function itself only renders with entries.
    }
}
