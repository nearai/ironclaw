//! `grep` engine, ported from the pinned
//! `packages/coding-agent/src/tools/grep.ts`, `path-utils.ts`
//! (parseSearchPath / splitPathAndSel), and `match-line-format.ts` at commit
//! `08819b279cf02ae2545e69dad7111ab48d91d35e`.
//!
//! `pattern` (required), semicolon-delimited `path` (default workspace
//! root), `skip` pagination, `case`, single-file line-range selectors
//! (`<file>:N-M`), hashline-mode match rows (`*N:line`) and context rows
//! (` N:line`), per-file caps, and the exact pinned error texts. Archives
//! and internal URLs are later slices.
//!
//! Documented deviations: regexes compile with the Rust `regex` crate, so
//! `Invalid regex: ${message}` renders the Rust parser's message (the
//! template itself is pinned); the native `gitignore` walk is a no-op on
//! virtual backends (no `.gitignore` rules exist there); the delimited-path
//! expansion beyond semicolons is not ported.

use std::{collections::BTreeSet, path::Path as FsPath};

use ironclaw_filesystem::{FileType, FilesystemError, FilesystemOperation};
use ironclaw_host_api::artifact::{
    ArtifactAccessError, ArtifactReadTarget, ArtifactRef, ArtifactSelector,
};
use serde_json::Value;

use super::super::config::{GREP_MAX_TOTAL_BYTES, MAX_READ_SIZE, MAX_VISITED_ENTRIES};
use super::hashline::format::format_hashline_header;
use super::selector::{LineRange, parse_line_ranges};
use super::state::CodingScopeKey;
use super::{
    CodingEngineContext, CodingEngineError, CodingEngineErrorKind, coding_error, display_path,
    filesystem_denied, input_error, read_limit_exceeded, resolve_input_path,
    workspace_virtual_root,
};

/// Scheme of the durable tool-output artifacts `read` already resolves.
///
/// The pinned `grep` accepts internal URLs as search inputs
/// (`parsePathSpecs`: "Internal URLs (`artifact://`, `skill://`, …) use the
/// URL-aware splitter"), searching the whole resource and honoring any embedded
/// line range as a match filter. Only `artifact://` is wired here because it is
/// the only internal scheme this port resolves; the others remain the later
/// slice this module's header describes.
const ARTIFACT_SCHEME: &str = "artifact://";

pub(super) fn is_artifact_url(path: &str) -> bool {
    path.len() > ARTIFACT_SCHEME.len()
        && path[..ARTIFACT_SCHEME.len()].eq_ignore_ascii_case(ARTIFACT_SCHEME)
}

const DEFAULT_FILE_LIMIT: usize = 20;
const MULTI_FILE_PER_FILE_MATCHES: usize = 20;
const SINGLE_FILE_MATCHES: usize = 200;
/// `grep.contextBefore` / `grep.contextAfter` defaults (settings-schema.ts).
const CONTEXT_BEFORE: usize = 1;
const CONTEXT_AFTER: usize = 3;

struct PathSpec {
    original: String,
    clean: String,
    ranges: Option<Vec<LineRange>>,
}

struct FileHits {
    display_path: String,
    snapshot_tag: Option<String>,
    lines: Vec<(u64, String, bool)>, // (line number, text, is_match)
}

#[derive(Default)]
struct GrepBudget {
    visited_entries: usize,
    bytes_scanned: u64,
}

pub(crate) async fn grep(
    ctx: &CodingEngineContext,
    input: Value,
) -> Result<String, CodingEngineError> {
    let Some(pattern) = input.get("pattern").and_then(Value::as_str) else {
        return Err(input_error("grep requires a string `pattern`"));
    };
    let raw_path = input.get("path").and_then(Value::as_str);
    let skip = input.get("skip");
    let case_sensitive = input.get("case").and_then(Value::as_bool);

    if pattern.trim().is_empty() {
        return Err(coding_error(
            CodingEngineErrorKind::PatternEmpty,
            "Pattern must not be empty",
        ));
    }
    let normalized_skip = match skip {
        None | Some(Value::Null) => 0usize,
        Some(Value::Number(number)) => {
            let value = number.as_f64().unwrap_or(f64::NAN);
            if !value.is_finite() || value < 0.0 {
                return Err(coding_error(
                    CodingEngineErrorKind::SkipNegative,
                    "Skip must be a non-negative number",
                ));
            }
            value.floor() as usize
        }
        Some(_) => {
            return Err(coding_error(
                CodingEngineErrorKind::SkipNegative,
                "Skip must be a non-negative number",
            ));
        }
    };

    let compiled = regex::RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive.unwrap_or(true))
        .build()
        .map_err(|error| {
            coding_error(
                CodingEngineErrorKind::InvalidRegex,
                format!("Invalid regex: {error}"),
            )
        })?;

    let scoped_paths = to_path_list(raw_path);
    let effective_paths: Vec<String> = if scoped_paths.is_empty() {
        vec![".".to_string()]
    } else {
        scoped_paths
    };

    let workspace_root = workspace_virtual_root(ctx).ok_or_else(|| {
        coding_error(
            CodingEngineErrorKind::PathResolution,
            "no workspace mount root".to_string(),
        )
    })?;

    // Parse each entry: peel `:N-M` selectors, prefer literal filesystem
    // matches.
    let mut specs: Vec<PathSpec> = Vec::new();
    for entry in &effective_paths {
        let (path_part, sel) = split_path_and_sel(entry);
        let mut clean = path_part.to_string();
        let mut ranges: Option<Vec<LineRange>> = None;
        if is_artifact_url(path_part) {
            // Upstream accepts a selector on an internal URL and searches the
            // whole resource, keeping any line range as a match filter. The
            // verbatim/index display modes (`raw`, `conflicts`) carry no meaning
            // for content search, so they are accepted and ignored rather than
            // rejected the way a filesystem path rejects them.
            if let Some(sel) = sel {
                match parse_line_ranges(sel) {
                    Ok(parsed) => ranges = parsed,
                    Err(message) => {
                        return Err(coding_error(
                            CodingEngineErrorKind::InvalidSelector,
                            message,
                        ));
                    }
                }
            }
            specs.push(PathSpec {
                original: entry.clone(),
                clean,
                ranges,
            });
            continue;
        }
        if let Some(sel) = sel {
            if literal_exists(ctx, entry).await? {
                clean = entry.clone();
            } else {
                let parsed = match parse_line_ranges(sel) {
                    Ok(Some(ranges)) => ranges,
                    Ok(None) => {
                        return Err(coding_error(
                            CodingEngineErrorKind::LineRangeSelectorRequiresSingleFile,
                            format!(
                                "path entry \"{entry}\" — only line-range selectors like \":50-100\" are supported (no \":raw\"/\":conflicts\")"
                            ),
                        ));
                    }
                    Err(message) => {
                        return Err(coding_error(
                            CodingEngineErrorKind::InvalidSelector,
                            message,
                        ));
                    }
                };
                if has_glob_path_chars(path_part) {
                    return Err(coding_error(
                        CodingEngineErrorKind::LineRangeSelectorRequiresSingleFile,
                        format!("Line-range selector requires a single file, not a glob: {entry}"),
                    ));
                }
                clean = path_part.to_string();
                ranges = Some(parsed);
            }
        }
        specs.push(PathSpec {
            original: entry.clone(),
            clean,
            ranges,
        });
    }

    // Internal-URL targets carry no filesystem path, so they are searched on
    // their own reader and never enter scope resolution, the line-range file
    // check, or the missing-path accounting below.
    let (artifact_specs, specs): (Vec<PathSpec>, Vec<PathSpec>) = specs
        .into_iter()
        .partition(|spec| is_artifact_url(&spec.clean));

    // Line-range selector targets must be single existing FILES (pinned
    // grep: the per-spec range check runs before generic path-missing
    // handling, so `gone.rs:1-5` reports the line-range message).
    for spec in &specs {
        if spec.ranges.is_none() {
            continue;
        }
        let Ok(resolved) = resolve_input_path(ctx, &spec.clean, FilesystemOperation::ReadFile)
        else {
            return Err(coding_error(
                CodingEngineErrorKind::LineRangePathNotFound,
                format!("Path not found for line-range selector: {}", spec.original),
            ));
        };
        let stat = stat_optional(ctx, &resolved.virtual_path).await?;
        let Some(stat) = stat else {
            return Err(coding_error(
                CodingEngineErrorKind::LineRangePathNotFound,
                format!("Path not found for line-range selector: {}", spec.original),
            ));
        };
        if stat.file_type != FileType::File {
            return Err(coding_error(
                CodingEngineErrorKind::LineRangeTargetIsDirectory,
                format!(
                    "Line-range selector requires a single file: {} is a directory",
                    spec.original
                ),
            ));
        }
    }

    // Resolve the scope: single entry vs multi-target.
    let mut missing_paths: Vec<String> = Vec::new();
    let mut resolved_targets: Vec<(String, Option<String>)> = Vec::new(); // (virtual path, glob)
    let mut is_directory = false;
    if specs.is_empty() {
        // Artifact-only search: there is no workspace scope to resolve.
    } else if specs.len() == 1 {
        let spec = &specs[0];
        let (base_path, glob_filter, has_glob) = parse_search_path(&spec.clean);
        let Ok(resolved) = resolve_input_path(ctx, &base_path, FilesystemOperation::ReadFile)
        else {
            return Err(coding_error(
                CodingEngineErrorKind::PathNotFound,
                format!("Path not found: {base_path}"),
            ));
        };
        let stat = stat_optional(ctx, &resolved.virtual_path).await?;
        let Some(stat) = stat else {
            let scope_path = display_path(&workspace_root, &resolved.virtual_path);
            return Err(coding_error(
                CodingEngineErrorKind::PathNotFound,
                format!("Path not found: {scope_path}"),
            ));
        };
        is_directory = stat.file_type == FileType::Directory;
        if !is_directory && !has_glob {
            resolved_targets.push((resolved.virtual_path.as_str().to_string(), None));
        } else {
            resolved_targets.push((
                resolved.virtual_path.as_str().to_string(),
                Some(glob_filter.unwrap_or_else(|| "**/*".to_string())),
            ));
        }
    } else {
        let mut valid: Vec<&PathSpec> = Vec::new();
        for spec in &specs {
            let (base_path, _, _) = parse_search_path(&spec.clean);
            if let Ok(resolved) = resolve_input_path(ctx, &base_path, FilesystemOperation::ReadFile)
                && stat_optional(ctx, &resolved.virtual_path).await?.is_some()
            {
                valid.push(spec);
                continue;
            }
            missing_paths.push(spec.original.clone());
        }
        if missing_paths.len() == specs.len() {
            return Err(coding_error(
                CodingEngineErrorKind::PathNotFoundMulti,
                format!(
                    "Path not found: {}; list each target in the semicolon-delimited `path`",
                    missing_paths.join(", ")
                ),
            ));
        }
        for spec in valid {
            let (base_path, glob_filter, has_glob) = parse_search_path(&spec.clean);
            let resolved = resolve_input_path(ctx, &base_path, FilesystemOperation::ReadFile)?;
            let stat = stat_optional(ctx, &resolved.virtual_path).await?;
            let is_file = stat
                .as_ref()
                .is_some_and(|stat| stat.file_type == FileType::File);
            if is_file && !has_glob {
                resolved_targets.push((resolved.virtual_path.as_str().to_string(), None));
            } else {
                resolved_targets.push((resolved.virtual_path.as_str().to_string(), glob_filter));
            }
        }
    }

    // Line-range selector targets are validated above (before scope
    // resolution) so missing paths surface the pinned line-range message.

    let is_multi_scope = resolved_targets.len() + artifact_specs.len() > 1 || is_directory;
    let per_file_match_cap = if is_multi_scope {
        MULTI_FILE_PER_FILE_MATCHES
    } else {
        SINGLE_FILE_MATCHES
    };

    // Collect hits per target file.
    let mut all_hits: Vec<FileHits> = Vec::new();
    let mut budget = GrepBudget::default();
    for (target, glob_filter) in &resolved_targets {
        let virtual_path =
            ironclaw_host_api::path::VirtualPath::new(target.clone()).map_err(|error| {
                coding_error(CodingEngineErrorKind::PathResolution, error.to_string())
            })?;
        let stat = stat_optional(ctx, &virtual_path).await?;
        let Some(stat) = stat else {
            continue;
        };
        if stat.sensitive {
            return Err(filesystem_denied());
        }
        if stat.file_type == FileType::File {
            let hits = search_file(
                ctx,
                &virtual_path,
                &workspace_root,
                &compiled,
                &specs,
                &mut budget,
                true,
            )
            .await?;
            all_hits.push(hits);
        } else if stat.file_type == FileType::Directory {
            let hits = search_directory(
                ctx,
                &virtual_path,
                glob_filter.as_deref().unwrap_or("**/*"),
                &workspace_root,
                &compiled,
                &specs,
                &mut budget,
            )
            .await?;
            all_hits.extend(hits);
        }
    }
    for spec in &artifact_specs {
        let hits = search_artifact(ctx, spec, &compiled, &mut budget).await?;
        all_hits.push(hits);
    }
    all_hits.sort_by(|a, b| a.display_path.cmp(&b.display_path));

    // Per-file match caps: `hits.lines` mixes match rows, context rows, and
    // `...` gap markers, so count rows where `is_match` is true (the pinned
    // caps are named for matches — upstream grep caps `matches`, not rows).
    for hits in &mut all_hits {
        let mut matches_seen = 0usize;
        let mut cut = hits.lines.len();
        let mut last_admitted_line = 0u64;
        for (index, (line_number, _, is_match)) in hits.lines.iter().enumerate() {
            if *is_match {
                matches_seen += 1;
                if matches_seen > per_file_match_cap {
                    cut = index;
                    break;
                }
                last_admitted_line = *line_number;
            }
        }
        // Drop everything beyond the last admitted match's trailing context
        // (the next match's leading context and any gap marker before it) so
        // no section ends mid-context or on `...`.
        while cut > 0 {
            let (line_number, _, _) = hits.lines[cut - 1];
            if line_number == 0
                || line_number > last_admitted_line.saturating_add(CONTEXT_AFTER as u64)
            {
                cut -= 1;
            } else {
                break;
            }
        }
        hits.lines.truncate(cut);
    }

    let total_files = all_hits.len();
    let can_paginate = is_multi_scope;
    let skip_files = if can_paginate {
        normalized_skip.min(total_files)
    } else {
        0
    };
    let selected: Vec<&FileHits> = if can_paginate {
        all_hits
            .iter()
            .skip(skip_files)
            .take(DEFAULT_FILE_LIMIT)
            .collect()
    } else {
        all_hits.iter().collect()
    };
    let file_limit_reached = can_paginate && total_files > skip_files + DEFAULT_FILE_LIMIT;
    let next_skip = skip_files + selected.len();
    let limit_message = if file_limit_reached {
        format!(
            "Showing files {}-{next_skip} of {total_files}. Use skip={next_skip} for the next page, or narrow paths/pattern.",
            skip_files + 1
        )
    } else {
        String::new()
    };

    let missing_paths_note = if missing_paths.is_empty() {
        None
    } else {
        Some(format!(
            "Skipped missing paths: {}",
            missing_paths.join(", ")
        ))
    };

    if selected.is_empty() {
        let skip_past_end =
            can_paginate && normalized_skip > 0 && total_files > 0 && skip_files >= total_files;
        let no_match_text = if skip_past_end {
            format!(
                "No more results ({total_files} files total; skip={normalized_skip} is past the end)"
            )
        } else {
            "No matches found".to_string()
        };
        return Ok(match missing_paths_note {
            Some(note) => format!("{no_match_text}\n{note}"),
            None => no_match_text,
        });
    }

    let is_grouped = is_directory || is_multi_scope;
    let mut output_lines: Vec<String> = Vec::new();
    if is_grouped {
        // formatGroupedFiles (pinned grouped-file-output.ts): prefix-folded
        // headers, one `#` per depth; a blank line precedes every directory
        // header and every root-level file header.
        let mut sections: Vec<(String, String, Vec<String>)> = Vec::new();
        for hits in &selected {
            let header_suffix = hits
                .snapshot_tag
                .as_ref()
                .map(|tag| format!("#{tag}"))
                .unwrap_or_default();
            sections.push((hits.display_path.clone(), header_suffix, render_hits(hits)));
        }
        let mut tree_root = GrepTree::new();
        for (path, suffix, body) in &sections {
            tree_root.insert(path, suffix.clone(), body.clone());
        }
        let mut emitted = false;
        tree_root.walk(&mut |kind, depth, name, body| {
            let hashes = "#".repeat(depth + 1);
            let needs_separator = emitted && (depth == 0 || kind == GrepEventKind::Dir);
            if needs_separator {
                output_lines.push(String::new());
            }
            emitted = true;
            match kind {
                GrepEventKind::Dir => {
                    output_lines.push(format!("{hashes} {name}/"));
                }
                GrepEventKind::File => {
                    output_lines.push(format!("{hashes} {name}"));
                    output_lines.extend(body.iter().cloned());
                }
            }
        });
    } else {
        for hits in &selected {
            if !output_lines.is_empty() {
                output_lines.push(String::new());
            }
            if let Some(tag) = &hits.snapshot_tag {
                output_lines.push(format_hashline_header(&hits.display_path, tag));
            } else if is_artifact_url(&hits.display_path) {
                // Name the resource without offering an edit anchor: artifacts
                // are immutable, so there is no snapshot tag to hand back.
                output_lines.push(format!("[{}]", hits.display_path));
            }
            output_lines.extend(render_hits(hits));
        }
    }

    if !limit_message.is_empty() {
        output_lines.push(String::new());
        output_lines.push(limit_message);
    }
    if let Some(note) = missing_paths_note {
        output_lines.push(String::new());
        output_lines.push(note);
    }
    Ok(output_lines.join("\n"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrepEventKind {
    Dir,
    File,
}

/// Prefix-folded path tree used by `formatGroupedFiles` (mirrors
/// `buildPathTree`/`walkPathTree` in the pinned `path-tree.ts`).
struct GrepTree {
    files: Vec<(String, String, Vec<String>)>, // (name, header suffix, body)
    subdirs: Vec<(String, GrepTree)>,
}

impl GrepTree {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            subdirs: Vec::new(),
        }
    }

    fn insert(&mut self, path: &str, suffix: String, body: Vec<String>) {
        let trimmed = path.trim_end_matches('/');
        let mut segments: Vec<&str> = trimmed.split('/').collect();
        let name = segments.pop().unwrap_or_default().to_string();
        let mut node = self;
        for segment in segments {
            let idx = node
                .subdirs
                .iter()
                .position(|(existing, _)| *existing == segment);
            if let Some(idx) = idx {
                node = &mut node.subdirs[idx].1;
            } else {
                // Index the entry just pushed instead of unwrapping
                // `last_mut`; the vector is non-empty by construction.
                let index = node.subdirs.len();
                node.subdirs.push((segment.to_string(), GrepTree::new()));
                node = &mut node.subdirs[index].1;
            }
        }
        node.files.push((name, suffix, body));
    }

    fn walk(&self, emit: &mut impl FnMut(GrepEventKind, usize, String, &Vec<String>)) {
        self.walk_at(0, emit);
    }

    fn walk_at(
        &self,
        depth: usize,
        emit: &mut impl FnMut(GrepEventKind, usize, String, &Vec<String>),
    ) {
        for (name, suffix, body) in &self.files {
            let header_name = format!("{name}{suffix}");
            emit(GrepEventKind::File, depth, header_name, body);
        }
        for (dir_name, subtree) in &self.subdirs {
            let mut parts: Vec<String> = vec![dir_name.clone()];
            let mut dir_node = subtree;
            while dir_node.files.is_empty() && dir_node.subdirs.len() == 1 {
                let (only_name, only_tree) = &dir_node.subdirs[0];
                parts.push(only_name.clone());
                dir_node = only_tree;
            }
            let folded = parts.join("/");
            emit(GrepEventKind::Dir, depth, folded, &Vec::new());
            dir_node.walk_at(depth + 1, emit);
        }
    }
}

fn to_path_list(input: Option<&str>) -> Vec<String> {
    let Some(input) = input else {
        return Vec::new();
    };
    if input.contains(';') {
        input.split(';').map(ToString::to_string).collect()
    } else {
        vec![input.to_string()]
    }
}

fn has_glob_path_chars(segment: &str) -> bool {
    segment.contains('*') || segment.contains('?') || segment.contains('[') || segment.contains('{')
}

/// `splitPathAndSel` (strict, no literal probe — the literal probe happens
/// in the caller via `literal_exists`).
fn split_path_and_sel(raw_path: &str) -> (&str, Option<&str>) {
    let Some(colon) = raw_path.rfind(':') else {
        return (raw_path, None);
    };
    if colon == 0 {
        return (raw_path, None);
    }
    let candidate = &raw_path[colon + 1..];
    if !selector_shaped(candidate) {
        return (raw_path, None);
    }
    let mut base_path = &raw_path[..colon];
    let mut sel = candidate;
    if let Some(inner_colon) = base_path.rfind(':')
        && inner_colon > 0
    {
        let inner_candidate = &base_path[inner_colon + 1..];
        let inner_is_raw = inner_candidate.eq_ignore_ascii_case("raw");
        let outer_is_raw = candidate.eq_ignore_ascii_case("raw");
        let inner_is_range = range_only(inner_candidate);
        let outer_is_range = range_only(candidate);
        if (inner_is_raw && outer_is_range) || (inner_is_range && outer_is_raw) {
            sel = &base_path[inner_colon + 1..];
            base_path = &base_path[..inner_colon];
        }
    }
    (base_path, Some(sel))
}

fn selector_shaped(candidate: &str) -> bool {
    if candidate.eq_ignore_ascii_case("raw") || candidate.eq_ignore_ascii_case("conflicts") {
        return true;
    }
    range_only(candidate)
}

fn range_only(candidate: &str) -> bool {
    if candidate.is_empty() {
        return false;
    }
    candidate.split(',').all(|chunk| {
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
    })
}

async fn literal_exists(
    ctx: &CodingEngineContext,
    raw_path: &str,
) -> Result<bool, CodingEngineError> {
    if let Ok(resolved) = resolve_input_path(ctx, raw_path, FilesystemOperation::ReadFile) {
        return Ok(stat_optional(ctx, &resolved.virtual_path).await?.is_some());
    }
    Ok(false)
}

/// `parseSearchPath` from the pinned `path-utils.ts`.
fn parse_search_path(file_path: &str) -> (String, Option<String>, bool) {
    let segments: Vec<&str> = file_path.split('/').collect();
    let mut first_glob_index = -1i64;
    for (index, segment) in segments.iter().enumerate() {
        if has_glob_path_chars(segment) {
            first_glob_index = index as i64;
            break;
        }
    }
    if first_glob_index == -1 {
        return (file_path.to_string(), None, false);
    }
    if first_glob_index == 0 {
        return (".".to_string(), Some(file_path.to_string()), true);
    }
    (
        segments[..first_glob_index as usize].join("/"),
        Some(segments[first_glob_index as usize..].join("/")),
        true,
    )
}

async fn stat_optional(
    ctx: &CodingEngineContext,
    path: &ironclaw_host_api::path::VirtualPath,
) -> Result<Option<ironclaw_filesystem::FileStat>, CodingEngineError> {
    match ctx.filesystem.stat(path).await {
        Ok(stat) => Ok(Some(stat)),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(coding_error(
            CodingEngineErrorKind::Filesystem,
            format!("filesystem error: {error}"),
        )),
    }
}

async fn read_file_text(
    ctx: &CodingEngineContext,
    virtual_path: &ironclaw_host_api::path::VirtualPath,
) -> Result<Option<String>, CodingEngineError> {
    let Some(versioned) = ctx.filesystem.get(virtual_path).await.map_err(|error| {
        coding_error(
            CodingEngineErrorKind::Filesystem,
            format!("filesystem error: {error}"),
        )
    })?
    else {
        return Ok(None);
    };
    match String::from_utf8(versioned.entry.body) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Ok(None),
    }
}

async fn search_file(
    ctx: &CodingEngineContext,
    virtual_path: &ironclaw_host_api::path::VirtualPath,
    workspace_root: &ironclaw_host_api::path::VirtualPath,
    compiled: &regex::Regex,
    specs: &[PathSpec],
    budget: &mut GrepBudget,
    explicit: bool,
) -> Result<FileHits, CodingEngineError> {
    let display = display_path(workspace_root, virtual_path);
    let Some(stat) = stat_optional(ctx, virtual_path).await? else {
        return Ok(FileHits {
            display_path: display,
            snapshot_tag: None,
            lines: Vec::new(),
        });
    };
    if stat.sensitive {
        if explicit {
            return Err(filesystem_denied());
        }
        return Ok(FileHits {
            display_path: display,
            snapshot_tag: None,
            lines: Vec::new(),
        });
    }
    if stat.len > MAX_READ_SIZE {
        if explicit {
            return Err(read_limit_exceeded());
        }
        return Ok(FileHits {
            display_path: display,
            snapshot_tag: None,
            lines: Vec::new(),
        });
    }
    let next_total = budget.bytes_scanned.saturating_add(stat.len);
    if next_total > GREP_MAX_TOTAL_BYTES {
        return Err(coding_error(
            CodingEngineErrorKind::ResourceLimit,
            "workspace grep exceeds the aggregate read limit",
        ));
    }
    budget.bytes_scanned = next_total;
    let Some(text) = read_file_text(ctx, virtual_path).await? else {
        return Ok(FileHits {
            display_path: display,
            snapshot_tag: None,
            lines: Vec::new(),
        });
    };
    let mut ranges = None;
    for spec in specs.iter().filter(|spec| spec.ranges.is_some()) {
        if let Ok(resolved) = resolve_input_path(ctx, &spec.clean, FilesystemOperation::ReadFile)
            && resolved.virtual_path == *virtual_path
        {
            ranges = spec.ranges.clone();
            break;
        }
    }
    let hits = scan_lines(&text, compiled, ranges.as_deref());
    let snapshot_tag = if hits.is_empty() {
        None
    } else {
        let normalized = super::hashline::normalize_to_lf(&text);
        Some(ctx.snapshots.record_and_return(
            &CodingScopeKey::from_scope(&ctx.scope, ctx.run_id),
            virtual_path.as_str(),
            &normalized,
        ))
    };
    Ok(FileHits {
        display_path: display,
        snapshot_tag,
        lines: hits,
    })
}

/// Scan one resource's text for matches plus the pinned context windows.
///
/// Shared by the filesystem and artifact search paths so both render identical
/// match/context/gap rows: `grep.contextBefore` = 1, `grep.contextAfter` = 3,
/// a `...` row at every gap, and an optional line-range match filter.
fn scan_lines(
    text: &str,
    compiled: &regex::Regex,
    ranges: Option<&[LineRange]>,
) -> Vec<(u64, String, bool)> {
    let lines: Vec<&str> = text.split('\n').collect();
    // Precompute match positions so context detection is linear.
    let match_lines: BTreeSet<u64> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| compiled.is_match(line))
        .map(|(index, _)| index as u64 + 1)
        .collect();
    let mut hits: Vec<(u64, String, bool)> = Vec::new();
    let mut last_emitted: Option<u64> = None;
    for (index, line) in lines.iter().enumerate() {
        let line_number = index as u64 + 1;
        if let Some(ranges) = ranges
            && !line_in_ranges(line_number, ranges)
        {
            continue;
        }
        let is_match = match_lines.contains(&line_number);
        if !is_match {
            // Per-direction context windows (pinned grep.contextBefore=1 /
            // contextAfter=3, settings-schema.ts): a non-match line is
            // emitted only inside the contextBefore window of a LATER match
            // or the contextAfter window of an EARLIER match. Lines are
            // visited in ascending order, so no line is ever repeated.
            let first_candidate = line_number.saturating_sub(CONTEXT_AFTER as u64);
            let last_candidate = line_number.saturating_add(CONTEXT_BEFORE as u64);
            let near_match = match_lines
                .range(first_candidate..=last_candidate)
                .next()
                .is_some();
            if !near_match {
                continue;
            }
        }
        if let Some(last) = last_emitted
            && line_number > last + 1
        {
            hits.push((0, "...".to_string(), false));
        }
        hits.push((line_number, (*line).to_string(), is_match));
        last_emitted = Some(line_number);
    }
    hits
}

/// Search one durable tool-output artifact.
///
/// The pinned grep resolves internal URLs through its router and searches the
/// whole resource, keeping any embedded line range as a match filter. Artifacts
/// are immutable and have no workspace path, so no Hashline snapshot is
/// recorded: an artifact is not editable, and handing back an edit anchor for
/// one would invite a stale-anchor edit against a file that never existed.
async fn search_artifact(
    ctx: &CodingEngineContext,
    spec: &PathSpec,
    compiled: &regex::Regex,
    budget: &mut GrepBudget,
) -> Result<FileHits, CodingEngineError> {
    let artifact_ref = spec
        .clean
        .parse::<ArtifactRef>()
        .map_err(|error| coding_error(CodingEngineErrorKind::PathResolution, error.to_string()))?;
    let reader = ctx.artifact_reader.as_ref().ok_or_else(|| {
        coding_error(
            CodingEngineErrorKind::Filesystem,
            "No session - artifacts unavailable".to_string(),
        )
    })?;
    // Same budget the bare `read` of an artifact uses, so a searchable window
    // is the same size as a readable one.
    let max_output_bytes = (super::read::DEFAULT_MAX_BYTES as u64)
        .max(super::read::DEFAULT_MAX_LINES * super::read::BYTES_PER_LINE_BUDGET as u64);
    let chunk = reader
        .read(ArtifactReadTarget {
            artifact_id: artifact_ref.id(),
            selector: ArtifactSelector::Full,
            max_output_bytes,
        })
        .await
        .map_err(|error| {
            let message = if error == ArtifactAccessError::OversizedUnsliced {
                format!(
                    "Artifact {} exceeds the search limit. Narrow it with a line range such as {}:1-2000, or read it in windows.",
                    artifact_ref.id().get(),
                    spec.clean
                )
            } else {
                error.to_string()
            };
            coding_error(CodingEngineErrorKind::Filesystem, message)
        })?
        .ok_or_else(|| {
            coding_error(
                CodingEngineErrorKind::PathNotFound,
                format!("Path not found: {}", spec.clean),
            )
        })?;
    let next_total = budget.bytes_scanned.saturating_add(chunk.total_bytes);
    if next_total > GREP_MAX_TOTAL_BYTES {
        return Err(coding_error(
            CodingEngineErrorKind::ResourceLimit,
            "workspace grep exceeds the aggregate read limit",
        ));
    }
    budget.bytes_scanned = next_total;
    let text = String::from_utf8(chunk.content).map_err(|_| {
        coding_error(
            CodingEngineErrorKind::Filesystem,
            format!(
                "[Cannot search binary artifact '{}'; binary bytes cannot be returned in JSON output.]",
                spec.clean
            ),
        )
    })?;
    Ok(FileHits {
        display_path: spec.clean.clone(),
        snapshot_tag: None,
        lines: scan_lines(&text, compiled, spec.ranges.as_deref()),
    })
}

async fn search_directory(
    ctx: &CodingEngineContext,
    dir: &ironclaw_host_api::path::VirtualPath,
    glob_filter: &str,
    workspace_root: &ironclaw_host_api::path::VirtualPath,
    compiled: &regex::Regex,
    specs: &[PathSpec],
    budget: &mut GrepBudget,
) -> Result<Vec<FileHits>, CodingEngineError> {
    let compiled_glob = glob::Pattern::new(glob_filter).map_err(|error| {
        coding_error(
            CodingEngineErrorKind::Input,
            format!("Invalid glob pattern: {error}"),
        )
    })?;
    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    let mut hits: Vec<FileHits> = Vec::new();
    let mut stack: Vec<ironclaw_host_api::path::VirtualPath> = vec![dir.clone()];
    while let Some(current) = stack.pop() {
        let entries = match ctx.filesystem.list_dir(&current).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => continue,
            Err(error) => {
                return Err(coding_error(
                    CodingEngineErrorKind::Filesystem,
                    format!("filesystem error: {error}"),
                ));
            }
        };
        for entry in entries {
            budget.visited_entries = budget.visited_entries.saturating_add(1);
            if budget.visited_entries > MAX_VISITED_ENTRIES {
                return Err(coding_error(
                    CodingEngineErrorKind::ResourceLimit,
                    "workspace traversal exceeds the entry limit",
                ));
            }
            if entry.name == ".git" || entry.name == "node_modules" {
                continue;
            }
            let stat = match stat_optional(ctx, &entry.path).await? {
                Some(stat) => stat,
                None => continue,
            };
            if stat.sensitive {
                continue;
            }
            // Match the pattern against the path relative to the walk base
            // (pinned: the native walker globs `pattern` under `searchPath`),
            // not the workspace-relative display path — `dir/*` must match
            // `a.ts` under `dir`, not `dir/a.ts`, with
            // `require_literal_separator`.
            let relative = display_path(dir, &entry.path);
            if entry.file_type == FileType::Directory {
                stack.push(entry.path.clone());
                continue;
            }
            if !compiled_glob.matches_path_with(FsPath::new(&relative), options) {
                continue;
            }
            let file_hits = search_file(
                ctx,
                &entry.path,
                workspace_root,
                compiled,
                specs,
                budget,
                false,
            )
            .await?;
            if !file_hits.lines.is_empty() {
                hits.push(file_hits);
            }
        }
    }
    Ok(hits)
}

fn line_in_ranges(line_number: u64, ranges: &[LineRange]) -> bool {
    ranges.iter().any(|range| {
        line_number >= range.start_line
            && (range.open_ended || line_number <= range.end_line.unwrap_or(range.start_line))
    })
}

/// `formatMatchLine` rows: `*N:line` matches, ` N:line` context, `...` gaps.
fn render_hits(hits: &FileHits) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for (line_number, text, is_match) in &hits.lines {
        if *line_number == 0 {
            rows.push("...".to_string());
            continue;
        }
        let marker = if *is_match { "*" } else { " " };
        rows.push(format!("{marker}{line_number}:{text}"));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_and_sel_grep_shapes() {
        assert_eq!(
            split_path_and_sel("src/foo.ts:50-100"),
            ("src/foo.ts", Some("50-100"))
        );
        assert_eq!(split_path_and_sel("src/foo.ts"), ("src/foo.ts", None));
        assert_eq!(
            split_path_and_sel("src/*.ts:50-100"),
            ("src/*.ts", Some("50-100"))
        );
    }

    #[test]
    fn parse_search_path_shapes() {
        assert_eq!(parse_search_path("src"), ("src".to_string(), None, false));
        assert_eq!(
            parse_search_path("src/*.ts"),
            ("src".to_string(), Some("*.ts".to_string()), true)
        );
        assert_eq!(
            parse_search_path("**/*.rs"),
            (".".to_string(), Some("**/*.rs".to_string()), true)
        );
    }

    #[test]
    fn line_in_ranges_checks() {
        let ranges = vec![LineRange {
            start_line: 50,
            end_line: Some(100),
            open_ended: false,
        }];
        assert!(line_in_ranges(50, &ranges));
        assert!(line_in_ranges(100, &ranges));
        assert!(!line_in_ranges(101, &ranges));
        let open = vec![LineRange {
            start_line: 50,
            end_line: None,
            open_ended: true,
        }];
        assert!(line_in_ranges(5000, &open));
    }

    #[test]
    fn match_rows_use_hashline_shapes() {
        let hits = FileHits {
            display_path: "a.ts".to_string(),
            snapshot_tag: None,
            lines: vec![
                (1, "fn main() {}".to_string(), false),
                (2, "match x {".to_string(), true),
                (0, "...".to_string(), false),
                (10, "// tail".to_string(), true),
            ],
        };
        let rows = render_hits(&hits);
        assert_eq!(
            rows,
            vec![" 1:fn main() {}", "*2:match x {", "...", "*10:// tail"]
        );
    }

    /// The selector splitter must not mistake an `artifact://N` authority for a
    /// selector, and must still peel a real range off one.
    #[test]
    fn split_path_and_sel_keeps_artifact_urls_intact() {
        assert_eq!(
            split_path_and_sel("artifact://3"),
            ("artifact://3", None),
            "the scheme's own colon is not a selector"
        );
        assert_eq!(
            split_path_and_sel("artifact://3:100-200"),
            ("artifact://3", Some("100-200"))
        );
        assert_eq!(
            split_path_and_sel("artifact://12:raw"),
            ("artifact://12", Some("raw"))
        );
    }

    #[test]
    fn artifact_urls_are_recognized_case_insensitively() {
        assert!(is_artifact_url("artifact://0"));
        assert!(is_artifact_url("ARTIFACT://0"));
        assert!(
            !is_artifact_url("artifact://"),
            "a scheme alone is not a ref"
        );
        assert!(!is_artifact_url("/workspace/artifact://0"));
        assert!(!is_artifact_url("skill://fix-ci"));
    }

    /// Both search paths share one scanner, so an artifact renders the same
    /// match/context/gap rows a file does.
    #[test]
    fn scan_lines_honors_ranges_and_context() {
        let text = (1..=40)
            .map(|n| {
                if n == 20 {
                    "needle\n".to_string()
                } else {
                    format!("line {n}\n")
                }
            })
            .collect::<String>();
        let compiled = regex::Regex::new("needle").expect("regex");

        let all = scan_lines(&text, &compiled, None);
        let matched: Vec<u64> = all
            .iter()
            .filter(|(_, _, is_match)| *is_match)
            .map(|(line, _, _)| *line)
            .collect();
        assert_eq!(matched, vec![20], "the single match is found");
        assert!(
            all.iter().any(|(line, _, _)| *line == 19),
            "contextBefore=1 emits the preceding line"
        );
        assert!(
            all.iter().any(|(line, _, _)| *line == 23),
            "contextAfter=3 emits the third following line"
        );
        assert!(
            !all.iter().any(|(line, _, _)| *line == 24),
            "contextAfter stops at 3"
        );

        // An embedded line range is a match filter, not a re-numbering.
        let filtered = scan_lines(
            &text,
            &compiled,
            Some(&[LineRange {
                start_line: 1,
                end_line: Some(10),
                open_ended: false,
            }]),
        );
        assert!(
            filtered.iter().all(|(_, _, is_match)| !*is_match),
            "a range that excludes the match yields no match rows"
        );
    }
}
