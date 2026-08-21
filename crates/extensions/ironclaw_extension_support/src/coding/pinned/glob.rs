//! `glob` engine, ported from the pinned
//! `packages/coding-agent/src/tools/glob.ts` and `path-utils.ts`
//! (parseFindPattern) at commit `08819b279cf02ae2545e69dad7111ab48d91d35e`.
//!
//! Semicolon-delimited `path` (defaults to the workspace mount root),
//! literal paths + glob patterns, `hidden`/`gitignore`/`limit` options, root
//! '/' denial, mtime-desc ranking, and the grouped-path output shape.
//!
//! IronClaw-specific deviations (documented): the backend is a virtual
//! `RootFilesystem`, so `.gitignore` rules do not exist on it — `gitignore`
//! is accepted and stored but never changes results; the native walker's
//! `node_modules` skip is mirrored unless the pattern mentions it; the glob
//! timeout path is not ported (no wall-clock budgets on virtual backends).

use std::path::Path as FsPath;

use ironclaw_filesystem::{FileType, FilesystemError, FilesystemOperation};
use serde_json::Value;

use super::super::config::MAX_VISITED_ENTRIES;
use super::{
    CodingEngineContext, CodingEngineError, CodingEngineErrorKind, coding_error, display_path,
    filesystem_denied, resolve_input_path, workspace_virtual_root,
};

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 200;

pub(crate) async fn glob(
    ctx: &CodingEngineContext,
    input: Value,
) -> Result<String, CodingEngineError> {
    let path_input = input.get("path").and_then(Value::as_str);
    let limit = input.get("limit").and_then(Value::as_f64);
    let hidden = input.get("hidden").and_then(Value::as_bool);
    let gitignore = input.get("gitignore").and_then(Value::as_bool);
    let _ = gitignore; // virtual backends carry no .gitignore rules (see module docs)

    let scoped_paths = to_path_list(path_input);
    let effective_paths: Vec<String> = if scoped_paths.is_empty() {
        vec![".".to_string()]
    } else {
        scoped_paths
    };
    let raw_patterns: Vec<String> = effective_paths
        .iter()
        .map(|path| path.trim().replace('\\', "/"))
        .collect();

    if raw_patterns
        .iter()
        .any(|pattern| !pattern.is_empty() && pattern.chars().all(|c| c == '/'))
    {
        return Err(coding_error(
            CodingEngineErrorKind::RootNotAllowed,
            "Searching from root directory '/' is not allowed",
        ));
    }
    if raw_patterns.iter().any(|pattern| pattern.is_empty()) {
        return Err(coding_error(
            CodingEngineErrorKind::EmptyPath,
            "`path` must contain non-empty globs or paths",
        ));
    }

    // Upstream `glob` routes internal URLs through its router but rejects glob
    // metacharacters on them outright (`Glob patterns are not supported for
    // internal URLs`, memory:// excepted). An artifact is one immutable blob,
    // not a tree, so there is nothing to walk — say so instead of resolving the
    // URL against the workspace and reporting a missing path.
    for pattern in &raw_patterns {
        if super::grep::is_artifact_url(pattern) {
            return Err(coding_error(
                CodingEngineErrorKind::Input,
                format!(
                    "Glob patterns are not supported for internal URLs: {pattern}. Use `read {pattern}` to inspect it, or `grep` to search it."
                ),
            ));
        }
    }

    let requested_limit = limit.unwrap_or(DEFAULT_LIMIT as f64);
    if !requested_limit.is_finite() || requested_limit <= 0.0 {
        return Err(coding_error(
            CodingEngineErrorKind::Input,
            "Limit must be a positive number",
        ));
    }
    let effective_limit = (requested_limit as usize).clamp(1, MAX_LIMIT);
    let include_hidden = hidden.unwrap_or(true);

    // Partition existing vs missing for multi-path calls.
    let mut missing_paths: Vec<String> = Vec::new();
    let mut effective_patterns = raw_patterns.clone();
    if raw_patterns.len() > 1 {
        let mut valid: Vec<String> = Vec::new();
        for pattern in &raw_patterns {
            if pattern_base_exists(ctx, pattern).await? {
                valid.push(pattern.clone());
            } else {
                missing_paths.push(pattern.clone());
            }
        }
        if valid.is_empty() {
            return Err(coding_error(
                CodingEngineErrorKind::PathNotFound,
                format!("Path not found: {}", missing_paths.join(", ")),
            ));
        }
        effective_patterns = valid;
    }

    let workspace_root = workspace_virtual_root(ctx).ok_or_else(|| {
        coding_error(
            CodingEngineErrorKind::PathResolution,
            "no workspace mount root".to_string(),
        )
    })?;

    // Resolve each pattern to (base virtual path, glob pattern, has_glob).
    let mut targets: Vec<GlobTarget> = Vec::new();
    for pattern in &effective_patterns {
        let (base_path, glob_pattern, has_glob) = parse_find_pattern(pattern);
        let resolved_base = resolve_pattern_base(ctx, &base_path).await?;
        let scope_path = display_path(&workspace_root, &resolved_base);
        targets.push(GlobTarget {
            search_path: resolved_base,
            glob_pattern,
            has_glob,
            scope_path,
        });
    }

    let is_single = targets.len() == 1;
    let scope_path = targets[0].scope_path.clone();

    // Run each target.
    let mut merged: Vec<(String, u64)> = Vec::new();
    for target in &targets {
        let stat = match ctx.filesystem.stat(&target.search_path).await {
            Ok(stat) => Some(stat),
            Err(FilesystemError::NotFound { .. }) => None,
            Err(error) => {
                return Err(coding_error(
                    CodingEngineErrorKind::Filesystem,
                    format!("filesystem error: {error}"),
                ));
            }
        };
        let Some(stat) = stat else {
            if is_single {
                return Err(coding_error(
                    CodingEngineErrorKind::PathNotFound,
                    format!("Path not found: {scope_path}"),
                ));
            }
            continue;
        };
        if stat.sensitive {
            if is_single && !target.has_glob {
                return Err(filesystem_denied());
            }
            continue;
        }
        if !target.has_glob && stat.file_type == FileType::File {
            let display = display_path(&workspace_root, &target.search_path);
            merged.push((display, mtime_ms(&stat)));
            continue;
        }
        if stat.file_type != FileType::Directory {
            if is_single {
                return Err(coding_error(
                    CodingEngineErrorKind::PathNotFound,
                    format!("Path not found: {scope_path}"),
                ));
            }
            continue;
        }
        let matches = walk_glob(
            ctx,
            &target.search_path,
            &target.glob_pattern,
            &workspace_root,
            include_hidden,
        )
        .await?;
        merged.extend(matches);
    }

    // Dedupe, rank by mtime desc, cap at the limit.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut ranked: Vec<(String, u64)> = Vec::new();
    for (path, mtime) in merged {
        if seen.insert(path.clone()) {
            ranked.push((path, mtime));
        }
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let files: Vec<String> = ranked
        .into_iter()
        .take(effective_limit)
        .map(|(path, _)| path)
        .collect();

    let missing_paths_note = if missing_paths.is_empty() {
        None
    } else {
        Some(format!(
            "Skipped missing paths: {}",
            missing_paths.join(", ")
        ))
    };

    if files.is_empty() {
        let mut parts: Vec<String> = vec!["No files found matching pattern".to_string()];
        if let Some(note) = missing_paths_note {
            parts.push(note);
        }
        return Ok(parts.join("\n"));
    }

    let base_output = format_grouped_paths(&files);
    let mut output = base_output;
    if let Some(note) = missing_paths_note {
        output.push_str("\n\n");
        output.push_str(&note);
    }
    Ok(output)
}

struct GlobTarget {
    search_path: ironclaw_host_api::path::VirtualPath,
    glob_pattern: String,
    has_glob: bool,
    scope_path: String,
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

/// `parseFindPattern` from the pinned `path-utils.ts`.
fn parse_find_pattern(pattern: &str) -> (String, String, bool) {
    let segments: Vec<&str> = pattern.split('/').collect();
    let mut first_glob_index = -1i64;
    for (index, segment) in segments.iter().enumerate() {
        if has_glob_path_chars(segment) {
            first_glob_index = index as i64;
            break;
        }
    }
    if first_glob_index == -1 {
        return (pattern.to_string(), "**/*".to_string(), false);
    }
    if first_glob_index == 0 {
        let needs_recursive = !pattern.starts_with("**/");
        return (
            ".".to_string(),
            if needs_recursive {
                format!("**/{pattern}")
            } else {
                pattern.to_string()
            },
            true,
        );
    }
    (
        segments[..first_glob_index as usize].join("/"),
        segments[first_glob_index as usize..].join("/"),
        true,
    )
}

fn has_glob_path_chars(segment: &str) -> bool {
    segment.contains('*') || segment.contains('?') || segment.contains('[') || segment.contains('{')
}

async fn pattern_base_exists(
    ctx: &CodingEngineContext,
    pattern: &str,
) -> Result<bool, CodingEngineError> {
    let (base_path, _, _) = parse_find_pattern(pattern);
    let Ok(resolved) = resolve_input_path(ctx, &base_path, FilesystemOperation::ListDir) else {
        return Ok(false);
    };
    match ctx.filesystem.stat(&resolved.virtual_path).await {
        Ok(stat) => Ok(!stat.sensitive),
        Err(FilesystemError::NotFound { .. }) => Ok(false),
        Err(error) => Err(coding_error(
            CodingEngineErrorKind::Filesystem,
            format!("filesystem error: {error}"),
        )),
    }
}

async fn resolve_pattern_base(
    ctx: &CodingEngineContext,
    base_path: &str,
) -> Result<ironclaw_host_api::path::VirtualPath, CodingEngineError> {
    let resolved = resolve_input_path(ctx, base_path, FilesystemOperation::ListDir)?;
    Ok(resolved.virtual_path)
}

fn mtime_ms(stat: &ironclaw_filesystem::FileStat) -> u64 {
    stat.modified
        .and_then(|modified| {
            modified
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .ok()
        })
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Recursive glob walk over the virtual filesystem. `require_literal_separator`
/// mirrors the pinned native walker: `dir/*` matches only DIRECT children
/// while `**/*.ts` recurses. `.git` is always skipped; `node_modules` is
/// skipped unless the pattern mentions it.
async fn walk_glob(
    ctx: &CodingEngineContext,
    base: &ironclaw_host_api::path::VirtualPath,
    pattern: &str,
    workspace_root: &ironclaw_host_api::path::VirtualPath,
    include_hidden: bool,
) -> Result<Vec<(String, u64)>, CodingEngineError> {
    const MAX_MATCHES: usize = 100_000;

    let compiled = glob::Pattern::new(pattern).map_err(|error| {
        coding_error(
            CodingEngineErrorKind::Input,
            format!("Invalid glob pattern: {error}"),
        )
    })?;
    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: !include_hidden,
    };
    let skip_node_modules = !pattern.contains("node_modules");

    let mut results: Vec<(String, u64)> = Vec::new();
    let mut stack: Vec<ironclaw_host_api::path::VirtualPath> = vec![base.clone()];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        if results.len() >= MAX_MATCHES {
            break;
        }
        let entries = match ctx.filesystem.list_dir(&dir).await {
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
            visited = visited.saturating_add(1);
            if visited > MAX_VISITED_ENTRIES {
                return Err(coding_error(
                    CodingEngineErrorKind::ResourceLimit,
                    "workspace traversal exceeds the entry limit",
                ));
            }
            let name = &entry.name;
            if name == ".git" {
                continue;
            }
            if skip_node_modules && name == "node_modules" {
                continue;
            }
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            let stat = match ctx.filesystem.stat(&entry.path).await {
                Ok(stat) => stat,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => {
                    tracing::debug!(path = entry.path.as_str(), %error, "skipping glob entry after stat failed");
                    continue;
                }
            };
            if stat.sensitive {
                continue;
            }
            // Match against the path relative to the walk base (the pinned
            // native walker globs `pattern` under `searchPath`), while the
            // reported match stays workspace-relative: `src/*.ts` must match
            // `a.ts` under `src` and report `src/a.ts`.
            let relative = display_path(base, &entry.path);
            let display_relative = display_path(workspace_root, &entry.path);
            let is_dir = entry.file_type == FileType::Directory;
            let matches_pattern = compiled.matches_path_with(FsPath::new(&relative), options);
            if matches_pattern {
                let mtime = mtime_ms(&stat);
                let display = if is_dir {
                    format!("{display_relative}/")
                } else {
                    display_relative.clone()
                };
                results.push((display, mtime));
            }
            if is_dir {
                stack.push(entry.path.clone());
            }
        }
    }
    Ok(results)
}

/// `formatGroupedPaths` from the pinned `packages/utils/src/path-tree.ts`:
/// a prefix-folded directory tree, one `#` per depth, files bare under the
/// deepest header, single-child chains folded.
fn format_grouped_paths(paths: &[String]) -> String {
    struct Node {
        files: Vec<(String, String)>, // (display name, full key)
        subdirs: Vec<Node>,
        name: String,
    }
    impl Node {
        fn new(name: String) -> Self {
            Self {
                files: Vec::new(),
                subdirs: Vec::new(),
                name,
            }
        }
    }

    let mut root = Node::new(String::new());
    for path in paths {
        let is_dir = path.ends_with('/');
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let segments: Vec<&str> = trimmed.split('/').collect();
        let mut node = &mut root;
        for segment in &segments[..segments.len() - 1] {
            let idx = node.subdirs.iter().position(|child| child.name == *segment);
            if let Some(idx) = idx {
                node = &mut node.subdirs[idx];
            } else {
                // Index the entry just pushed instead of unwrapping
                // `last_mut`; the vector is non-empty by construction.
                let index = node.subdirs.len();
                node.subdirs.push(Node::new(segment.to_string()));
                node = &mut node.subdirs[index];
            }
        }
        let name = segments.last().copied().unwrap_or_default();
        if is_dir {
            let idx = node.subdirs.iter().position(|child| child.name == name);
            if idx.is_none() {
                node.subdirs.push(Node::new(name.to_string()));
            }
        } else {
            node.files.push((name.to_string(), path.clone()));
        }
    }

    fn walk(node: &Node, depth: usize, lines: &mut Vec<String>) {
        for (name, _) in &node.files {
            lines.push(name.clone());
        }
        for subdir in &node.subdirs {
            let mut parts: Vec<String> = vec![subdir.name.clone()];
            let mut dir_node = subdir;
            while dir_node.files.is_empty() && dir_node.subdirs.len() == 1 {
                let only = &dir_node.subdirs[0];
                parts.push(only.name.clone());
                dir_node = only;
            }
            lines.push(format!("{} {}/", "#".repeat(depth + 1), parts.join("/")));
            walk(dir_node, depth + 1, lines);
        }
    }

    let mut lines: Vec<String> = Vec::new();
    walk(&root, 0, &mut lines);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_find_pattern_shapes() {
        assert_eq!(
            parse_find_pattern("src/**/*.ts"),
            ("src".to_string(), "**/*.ts".to_string(), true)
        );
        assert_eq!(
            parse_find_pattern("*.ts"),
            (".".to_string(), "**/*.ts".to_string(), true)
        );
        assert_eq!(
            parse_find_pattern("**/*.json"),
            (".".to_string(), "**/*.json".to_string(), true)
        );
        assert_eq!(
            parse_find_pattern("src/app"),
            ("src/app".to_string(), "**/*".to_string(), false)
        );
    }

    #[test]
    fn to_path_list_splits_semicolons() {
        assert_eq!(to_path_list(None), Vec::<String>::new());
        assert_eq!(to_path_list(Some("a;b")), vec!["a", "b"]);
        assert_eq!(to_path_list(Some("a")), vec!["a"]);
    }

    #[test]
    fn numeric_limit_accepts_json_float_representation() {
        let input = serde_json::json!({"limit": 1.0});

        assert_eq!(input.get("limit").and_then(Value::as_f64), Some(1.0));
        assert_eq!(
            input
                .get("limit")
                .and_then(Value::as_f64)
                .map(|limit| (limit as usize).clamp(1, MAX_LIMIT)),
            Some(1)
        );
    }

    #[test]
    fn grouped_paths_shape() {
        let paths = vec!["src/a.ts".to_string(), "src/b.ts".to_string()];
        assert_eq!(format_grouped_paths(&paths), "# src/\na.ts\nb.ts");
        let paths = vec!["a/b/c.ts".to_string()];
        assert_eq!(format_grouped_paths(&paths), "# a/b/\nc.ts");
        let paths = vec!["tests/".to_string(), "src/a.ts".to_string()];
        // Groups follow first-seen input order (pinned `walkPathTree`):
        // `tests/` is a root-level directory leaf and appears before `src/`.
        assert_eq!(format_grouped_paths(&paths), "# tests/\n# src/\na.ts");
    }
}
