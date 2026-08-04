//! Shared scanner machinery for the §10 anti-slippage ratchets.
//!
//! One hardened implementation of the walk/strip/match pipeline, so every
//! ratchet gets the same guarantees: comment/string stripping, restricted
//! visibility (`pub(crate)`/`pub(super)`/`pub(in …)`), optional `unsafe`/`auto`
//! modifiers, occurrence-preserving scans (same-file duplicates stay visible),
//! and a production-scoped walk (skips `target/`, `tests/`, `examples/`,
//! `benches/`). The scanners are line-based, not cfg-aware: a pub-visible
//! definition in an inline `#[cfg(test)]` module in src IS inventoried — keep
//! test doubles under `tests/` (or justify an allowlist entry in review).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One matched definition: where it was found and whether the definition line
/// sits under a `#[cfg(...)]` attribute (mutually exclusive compile branches —
/// e.g. the durable/no-durable alias pairs in composition's `factory.rs` — are
/// legitimate same-name definitions, not duplicate debt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDefOccurrence {
    pub path: PathBuf,
    pub cfg_gated: bool,
}

/// The workspace root: the nearest ancestor of `start` holding both a `crates/`
/// directory and a `Cargo.toml`.
///
/// Deliberately a search, not `CARGO_MANIFEST_DIR.parent().parent()`. The fixed
/// two-level form encoded "this crate sits directly under `crates/`", which the
/// family move (`crates/<family>/ironclaw_*`, PROPOSAL §5) makes false — and its
/// failure is silent for any gate that walks a directory: a wrong root makes
/// `root.join("crates")` resolve to `crates/crates`, the walk finds nothing, and
/// the gate passes having scanned zero files (CHECKLIST WS0, #6963).
///
/// This is the crate's ONLY definition of the rule — all twelve private copies
/// were deleted in its favour, including
/// `reborn_registration_pipeline_boundary.rs`'s (a review catch on #6996: it
/// had been left behind with a comment claiming it needed one, which was never
/// true — nothing about resolving scopes prevents a test binary from declaring
/// `mod ratchet_support`).
#[allow(dead_code)]
pub fn find_workspace_root(start: &Path) -> Result<PathBuf, String> {
    let mut current = Some(start);
    while let Some(directory) = current {
        if directory.join("crates").is_dir() && directory.join("Cargo.toml").is_file() {
            return Ok(directory.to_path_buf());
        }
        current = directory.parent();
    }
    Err(format!(
        "no workspace root above {} — expected an ancestor holding both crates/ and Cargo.toml",
        start.display()
    ))
}

pub fn workspace_root() -> PathBuf {
    find_workspace_root(Path::new(env!("CARGO_MANIFEST_DIR")))
        .expect("architecture crate must sit inside the workspace")
}

// ---------------------------------------------------------------------------
// Crate inventory: the Rust half of the WS10 path-keying rule
//
// Every gate in this directory that names a crate spells it the flat way —
// `crates/ironclaw_llm/src/lib.rs`. The family move (`crates/<family>/…`,
// PROPOSAL §5) makes that spelling false for ~60 crates at once, and a gate
// that resolves it with a bare `root.join(…)` then either scans a directory
// that is not there or asserts against an allowlist nothing can match.
//
// The rule these helpers implement is the one `scripts/ci/lib/crate_tree.py`
// states for the Python-side gates, so the two inventories agree by
// construction and `reborn_crate_inventory.rs` pins them equal:
//
//   * a crate directory is the OUTERMOST directory under `crates/` owning a
//     `Cargo.toml`;
//   * a manifest declaring its own `[workspace]` table is the root of a
//     DIFFERENT workspace (`crates/ironclaw_silk_decoder`, the six `wasm-src`
//     guests) and is not a crate of this one;
//   * `target/` and dotted directories are skipped;
//   * fewer than `MIN_CRATE_DIRECTORIES` results is a broken checkout, not an
//     answer — refuse rather than let a gate report success on an empty scan.
//
// A gate keeps its literal, readable `crates/<crate>/…` spelling and resolves
// it through `crate_path`; the literal becomes a crate NAME plus an in-crate
// remainder rather than a directory path. On today's flat tree resolution is
// the identity, which is what makes adopting it a behavior-free change.
// ---------------------------------------------------------------------------

/// Fail-closed floor, mirroring `crate_tree.py`'s. Far below the real count
/// (65 at the WS0 baseline); a handful of results always means the walk lost
/// the tree rather than that the tree shrank.
pub const MIN_CRATE_DIRECTORIES: usize = 20;

const CRATES_ROOT: &str = "crates";
const WORKSPACE_TABLE_HEADER: &str = "[workspace]";

/// Every crate directory under `crates/`, repo-relative, POSIX-separated,
/// sorted. Errors carry the reason so the caller can panic with it.
#[allow(dead_code)]
pub fn try_crate_directories(root: &Path) -> Result<Vec<String>, String> {
    let crates_root = root.join(CRATES_ROOT);
    if !crates_root.is_dir() {
        return Err(format!(
            "no {CRATES_ROOT}/ directory under {} — crate discovery cannot run. Every \
             path-keyed gate resolves its scope through this inventory; refusing rather \
             than scanning nothing is deliberate \
             (docs/reborn/target-architecture/CHECKLIST.md WS10).",
            root.display()
        ));
    }

    let mut manifests = Vec::new();
    collect_crate_manifest_dirs(root, &crates_root, &mut manifests)?;
    // Shallowest first so the outermost owner of a path is always seen before
    // any manifest nested inside it.
    manifests.sort_by(|left, right| {
        (left.matches('/').count(), left.as_str())
            .cmp(&(right.matches('/').count(), right.as_str()))
    });

    let mut directories: Vec<String> = Vec::new();
    for candidate in manifests {
        if directories
            .iter()
            .any(|kept| candidate == *kept || candidate.starts_with(&format!("{kept}/")))
        {
            continue;
        }
        directories.push(candidate);
    }

    if directories.len() < MIN_CRATE_DIRECTORIES {
        return Err(format!(
            "crate discovery found only {} crate director(ies) under {} (floor is {}). \
             Either the crate tree moved out from under this gate or the workspace root is \
             wrong. Failing closed: a gate that scans nothing must never report success \
             (docs/reborn/target-architecture/CHECKLIST.md WS10).",
            directories.len(),
            crates_root.display(),
            MIN_CRATE_DIRECTORIES
        ));
    }

    directories.sort();
    Ok(directories)
}

/// `try_crate_directories`, panicking on refusal.
#[allow(dead_code)]
pub fn crate_directories(root: &Path) -> Vec<String> {
    try_crate_directories(root).unwrap_or_else(|error| panic!("{error}"))
}

/// Directories under `crates/` that are roots of a *separate* workspace, and so
/// are excluded from the inventory by construction rather than by omission.
#[allow(dead_code)]
pub fn nested_workspace_roots(root: &Path) -> Vec<String> {
    let mut roots = Vec::new();
    let crates_root = root.join(CRATES_ROOT);
    if crates_root.is_dir() {
        let _ = walk_manifest_dirs(root, &crates_root, &mut |relative, declares_workspace| {
            if declares_workspace {
                roots.push(relative.to_string());
            }
        });
    }
    roots.sort();
    roots
}

fn collect_crate_manifest_dirs(
    root: &Path,
    directory: &Path,
    out: &mut Vec<String>,
) -> Result<(), String> {
    walk_manifest_dirs(root, directory, &mut |relative, declares_workspace| {
        if !declares_workspace {
            out.push(relative.to_string());
        }
    })
}

fn walk_manifest_dirs(
    root: &Path,
    directory: &Path,
    visit: &mut dyn FnMut(&str, bool),
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            walk_manifest_dirs(root, &path, visit)?;
            continue;
        }
        if name != "Cargo.toml" {
            continue;
        }
        // Read fail-closed, like `crate_tree.py`: an unreadable manifest counts
        // as a normal crate so a permissions problem surfaces as a build error
        // rather than as a directory quietly leaving every path-keyed gate.
        let declares_workspace = std::fs::read_to_string(&path)
            .map(|text| {
                text.lines()
                    .any(|line| line.trim() == WORKSPACE_TABLE_HEADER)
            })
            .unwrap_or(false);
        let relative = relative_posix(root, directory);
        visit(&relative, declares_workspace);
    }
    Ok(())
}

fn relative_posix(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The crate directory whose basename is `name`, repo-relative.
///
/// Absent or ambiguous is an error, never a silent empty scope: a gate that
/// names a crate must fail at the name it can no longer resolve.
#[allow(dead_code)]
pub fn try_crate_directory(root: &Path, name: &str) -> Result<String, String> {
    let inventory = try_crate_directories(root)?;
    let matches: Vec<&String> = inventory
        .iter()
        .filter(|directory| directory.rsplit('/').next() == Some(name))
        .collect();
    match matches.as_slice() {
        [only] => Ok((*only).clone()),
        found => Err(format!(
            "expected exactly one crate directory named {name:?} under {CRATES_ROOT}/, found \
             {}: {found:?}. If the crate was renamed or removed, repoint the gate that names \
             it rather than letting it measure an empty tree.",
            found.len()
        )),
    }
}

/// `try_crate_directory`, panicking on refusal.
#[allow(dead_code)]
pub fn crate_directory(root: &Path, name: &str) -> String {
    try_crate_directory(root, name).unwrap_or_else(|error| panic!("{error}"))
}

/// Absolute path of the crate directory named `name`.
#[allow(dead_code)]
pub fn crate_dir(root: &Path, name: &str) -> PathBuf {
    root.join(crate_directory(root, name))
}

/// Rewrite a *logical* `crates/<crate>/<remainder>` spelling into the path the
/// crate actually occupies, resolving `<crate>` through the inventory.
///
/// Order matters, and it is what makes adoption behavior-free:
///
///   1. the spec already names a real crate directory → identity (this is the
///      only branch taken on today's flat tree, and the branch that keeps an
///      already-nested crate such as `crates/extensions/ironclaw_extension_support`
///      spelled the way it really sits);
///   2. the spec resolves to something that exists but is owned by no crate
///      (`crates/AGENTS.md`, a data-only package directory) → identity;
///   3. the crate moved → resolve the first segment as a crate directory
///      basename and rejoin the remainder;
///   4. nothing resolves → refuse. A spec naming a crate that no longer exists
///      is a stale entry, and answering it with a path that matches nothing is
///      the silent failure this row exists to kill.
///
/// A trailing `/` is preserved, so allowlist entries used as `contains`
/// fragments (`crates/ironclaw_llm/src/`) resolve to fragments.
#[allow(dead_code)]
pub fn try_resolve_crate_relative(root: &Path, logical: &str) -> Result<String, String> {
    let prefix = format!("{CRATES_ROOT}/");
    if !logical.starts_with(&prefix) {
        return Ok(logical.to_string());
    }
    let inventory = try_crate_directories(root)?;
    for directory in &inventory {
        if logical == directory || logical.starts_with(&format!("{directory}/")) {
            return Ok(logical.to_string());
        }
    }
    if root.join(logical.trim_end_matches('/')).exists() {
        return Ok(logical.to_string());
    }
    let rest = &logical[prefix.len()..];
    let (name, remainder) = match rest.split_once('/') {
        Some((name, remainder)) => (name, Some(remainder)),
        None => (rest, None),
    };
    let directory = try_crate_directory(root, name).map_err(|error| {
        format!(
            "path {logical:?} names crate {name:?}, which does not resolve against the crate \
             inventory and does not exist on disk — repoint the gate that names it rather \
             than leaving a term that matches nothing ({error})"
        )
    })?;
    Ok(match remainder {
        Some(remainder) => format!("{directory}/{remainder}"),
        None => directory,
    })
}

/// `try_resolve_crate_relative`, panicking on refusal.
#[allow(dead_code)]
pub fn resolve_crate_relative(root: &Path, logical: &str) -> String {
    try_resolve_crate_relative(root, logical).unwrap_or_else(|error| panic!("{error}"))
}

/// Absolute path for a logical `crates/<crate>/<remainder>` spelling —
/// the drop-in replacement for `root.join("crates/ironclaw_x/…")`.
#[allow(dead_code)]
pub fn crate_path(root: &Path, logical: &str) -> PathBuf {
    root.join(resolve_crate_relative(root, logical))
}

/// The directory basename of the crate owning `path`, or `""` when no crate
/// under `crates/` owns it.
///
/// The drop-in replacement for "take the first path component under
/// `crates/`". That idiom answers `substrates` once a crate moves into a
/// family directory, and already answers `extensions` for the two crates that
/// sit one level down today — so a gate keyed on it compares an ownership
/// question against a directory name and silently attributes code to the wrong
/// owner. Outermost-wins, exactly like the inventory.
#[allow(dead_code)]
pub fn owning_crate_name(root: &Path, path: &Path) -> String {
    let Ok(relative) = path.strip_prefix(root) else {
        return String::new();
    };
    let relative = relative.to_string_lossy().replace('\\', "/");
    for directory in crate_directories(root) {
        if relative.starts_with(&format!("{directory}/")) || relative == directory {
            return directory
                .rsplit('/')
                .next()
                .unwrap_or(&directory)
                .to_string();
        }
    }
    String::new()
}

/// Names with more than one defining occurrence — a second same-named
/// definition elsewhere is new debt hiding behind an allowlist entry (§10) —
/// EXCEPT when every occurrence is `#[cfg(...)]`-gated (mutually exclusive
/// compile branches of the same type, the factory durable/no-durable pattern).
/// A mix of gated and ungated occurrences is still flagged.
#[allow(dead_code)]
pub fn duplicate_definitions(
    found: &BTreeMap<String, Vec<TypeDefOccurrence>>,
) -> Vec<(&str, &Vec<TypeDefOccurrence>)> {
    found
        .iter()
        .filter(|(_, occurrences)| {
            occurrences.len() > 1 && !occurrences.iter().all(|occ| occ.cfg_gated)
        })
        .map(|(name, occurrences)| (name.as_str(), occurrences))
        .collect()
}

/// Walk `dir` recursively, scanning every production `.rs` file for pub-visible
/// type definitions introduced by one of `keywords` (e.g. `"struct "`,
/// `"type "`) whose identifier satisfies `matches`. Records every occurrence
/// (identifier → defining file, once per occurrence). Skips `target/`,
/// `tests/`, `examples/`, and `benches/` trees plus any file named in
/// `skip_files` (the ratchet files themselves, as defense in depth — their
/// fixtures are already excluded by string stripping).
#[allow(dead_code)]
pub fn collect_type_defs(
    dir: &Path,
    keywords: &[&str],
    matches: &dyn Fn(&str) -> bool,
    skip_files: &[&str],
    out: &mut BTreeMap<String, Vec<TypeDefOccurrence>>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str());
            if matches!(dir_name, Some("target" | "tests" | "examples" | "benches")) {
                continue;
            }
            collect_type_defs(&path, keywords, matches, skip_files, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && skip_files.contains(&name)
        {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for (ident, cfg_gated) in scan_type_defs(&contents, keywords, matches) {
            out.entry(ident).or_default().push(TypeDefOccurrence {
                path: path.clone(),
                cfg_gated,
            });
        }
    }
}

/// Extract the identifier from every pub-visible type definition introduced by
/// one of `keywords` — `pub`, `pub(crate)`, `pub(super)`, or `pub(in path)`,
/// with optional `unsafe`/`auto` modifiers (e.g. `pub unsafe trait …`).
/// Comments and string literals are stripped first, so definition-shaped text
/// inside them is not matched. Matches the definition form, not references.
/// Returns every occurrence in source order (no dedup) so same-file duplicate
/// definitions in different modules stay visible to the multiplicity check.
/// The `bool` per occurrence is whether the definition sits under a
/// (single-line) `#[cfg(...)]` attribute in its immediately preceding attribute
/// block — used to exempt mutually exclusive compile branches from the
/// duplicate check.
#[allow(dead_code)]
pub fn scan_type_defs(
    source: &str,
    keywords: &[&str],
    matches: &dyn Fn(&str) -> bool,
) -> Vec<(String, bool)> {
    let stripped = strip_comments_and_strings(source);
    let mut out = Vec::new();
    // `#[...]` attributes immediately preceding the current line; reset by any
    // other non-blank line. Multi-line attributes (rustfmt splits long
    // `#[cfg(any(...))]` gates) are tracked by square-bracket balance — strings
    // are already stripped, so bracket counting is safe.
    let mut pending_attrs: Vec<String> = Vec::new();
    let mut attr_bracket_depth: usize = 0;
    for line in stripped.lines() {
        let trimmed = line.trim_start();
        if attr_bracket_depth > 0 {
            // Continuation of a multi-line attribute.
            attr_bracket_depth = (attr_bracket_depth + trimmed.matches('[').count())
                .saturating_sub(trimmed.matches(']').count());
            continue;
        }
        if trimmed.starts_with("#[") {
            pending_attrs.push(trimmed.to_string());
            attr_bracket_depth = trimmed
                .matches('[')
                .count()
                .saturating_sub(trimmed.matches(']').count());
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let cfg_gated = pending_attrs.iter().any(|attr| attr.starts_with("#[cfg("));
        pending_attrs.clear();
        let Some(after_pub) = trimmed.strip_prefix("pub") else {
            continue;
        };
        // Optional restricted-visibility qualifier: `(crate)`, `(super)`, `(in path)`.
        let mut rest = match after_pub.trim_start().strip_prefix('(') {
            Some(inner) => match inner.split_once(')') {
                Some((_, tail)) => tail,
                None => continue,
            },
            None => after_pub,
        }
        .trim_start();
        // Optional declaration modifiers before the keyword.
        loop {
            let mut advanced = false;
            for modifier in ["unsafe ", "auto "] {
                if let Some(tail) = rest.strip_prefix(modifier) {
                    rest = tail.trim_start();
                    advanced = true;
                }
            }
            if !advanced {
                break;
            }
        }
        let Some(after_kw) = keywords.iter().find_map(|kw| rest.strip_prefix(kw)) else {
            continue;
        };
        let ident: String = after_kw
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if matches(&ident) {
            out.push((ident, cfg_gated));
        }
    }
    out
}

/// Whether stripped `code` names the crate `krate` as a whole token — i.e.
/// not as a prefix of a longer identifier. `ironclaw_product::X` and
/// `use ironclaw_product as p;` both match for `ironclaw_product`;
/// `ironclaw_product_contracts::X` does not, because the token continues with
/// an identifier character. A plain `contains("{krate}::")` misses the
/// `use … as alias;` and `extern crate … as alias;` forms, through which the
/// dependency would keep compiling while the scan reports it gone.
#[allow(dead_code)]
pub fn names_crate(code: &str, krate: &str) -> bool {
    let bytes = code.as_bytes();
    let mut search = 0;
    while let Some(pos) = code[search..].find(krate) {
        let at = search + pos;
        search = at + 1;
        let before_ok = at == 0 || {
            let ch = bytes[at - 1];
            !(ch.is_ascii_alphanumeric() || ch == b'_')
        };
        let end = at + krate.len();
        let after_ok = end >= bytes.len() || {
            let ch = bytes[end];
            !(ch.is_ascii_alphanumeric() || ch == b'_')
        };
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Out-of-line module declarations (`mod name;`) in stripped source, each with
/// whether the declaration's immediately preceding attribute run contains
/// `cfg(test)`. Inline modules (`mod name { … }`) are not returned — their
/// contents live in the same file and are the brace-stripper's problem.
///
/// The gate is read by walking **backwards** from the `mod` keyword, so the
/// visibility qualifier that may sit between the attribute run and `mod` has to
/// be stepped over first ([`strip_trailing_visibility`]). Not doing so ended the
/// run at `pub` and reported `#[cfg(test)] pub mod x;` as **ungated** — the
/// fail-open direction, since [`cfg_test_only_files`] would then leave a
/// test-only file classified as production and a test double in a
/// production-named file could satisfy a residue row or an implementor pin.
/// Raised by @serrrfirat on #7000.
#[allow(dead_code)]
pub fn out_of_line_mod_decls(stripped: &str) -> Vec<(String, bool)> {
    let bytes = stripped.as_bytes();
    let mut decls = Vec::new();
    let mut search = 0;
    while let Some(pos) = stripped[search..].find("mod ") {
        let at = search + pos;
        search = at + "mod ".len();
        if at > 0 {
            let prev = bytes[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }
        let rest = &stripped[at + "mod ".len()..];
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        if !rest[name.len()..].trim_start().starts_with(';') {
            continue;
        }
        let before = strip_trailing_visibility(&stripped[..at]);
        let gated = trailing_attribute_run_contains(before, "cfg(test)");
        decls.push((name, gated));
    }
    decls
}

/// `#[path = "…"] mod name;` declarations in **raw** source (the path lives in
/// a string literal, which the stripper blanks), mapping module name to its
/// explicit file path. The scan tolerates further attributes between the
/// `path` attribute and the `mod` keyword.
#[allow(dead_code)]
pub fn explicit_mod_paths(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut search = 0;
    while let Some(pos) = raw[search..].find("#[path") {
        let at = search + pos;
        search = at + "#[path".len();
        let rest = &raw[at..];
        let Some(open_quote) = rest.find('"') else {
            continue;
        };
        let after_quote = &rest[open_quote + 1..];
        let Some(close_quote) = after_quote.find('"') else {
            continue;
        };
        let path = &after_quote[..close_quote];
        // Walk forward past whitespace and further attributes to `mod name;`.
        let mut tail = after_quote[close_quote + 1..].trim_start();
        tail = tail.strip_prefix(']').unwrap_or(tail).trim_start();
        while tail.starts_with("#[") {
            let Some(close) = tail.find(']') else { break };
            tail = tail[close + 1..].trim_start();
        }
        for visibility in ["pub(crate) ", "pub(super) ", "pub "] {
            tail = tail.strip_prefix(visibility).unwrap_or(tail).trim_start();
        }
        let Some(after_mod) = tail.strip_prefix("mod ") else {
            continue;
        };
        let name: String = after_mod
            .trim_start()
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            out.insert(name, path.to_string());
        }
    }
    out
}

/// Strip one trailing visibility qualifier — `pub`, `pub(crate)`,
/// `pub(super)`, `pub(in path)` — from the end of `before`, so a backwards
/// attribute-run walk starts at the true beginning of the declaration rather
/// than stopping dead at `pub`. Returns `before` (trimmed) unchanged when the
/// tail is not a visibility qualifier, so nothing else can be consumed by
/// accident: the parenthesised form is only stripped when the group balances
/// *and* is immediately preceded by the whole token `pub`.
fn strip_trailing_visibility(before: &str) -> &str {
    let trimmed = before.trim_end();
    // `pub(crate)` / `pub(super)` / `pub(in a::b)` — step over the group first.
    let head = match trimmed.strip_suffix(')') {
        Some(inner) => {
            let bytes = inner.as_bytes();
            let mut depth = 1usize;
            let mut index = inner.len();
            while index > 0 {
                index -= 1;
                match bytes[index] {
                    b')' => depth += 1,
                    b'(' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth != 0 {
                return trimmed;
            }
            inner[..index].trim_end()
        }
        None => trimmed,
    };
    // `pub` must be a whole token: not the tail of an identifier, and not the
    // raw form `r#pub`.
    match head.strip_suffix("pub") {
        Some(rest)
            if rest
                .chars()
                .next_back()
                .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == '#')) =>
        {
            rest
        }
        _ => trimmed,
    }
}

/// Whether the contiguous run of `#[…]` attributes ending at the end of
/// `before` contains `needle`. Strings are already stripped, so `#[` inside a
/// literal cannot mislead the backwards walk.
fn trailing_attribute_run_contains(before: &str, needle: &str) -> bool {
    let mut rest = before.trim_end();
    while rest.ends_with(']') {
        let Some(open) = rest.rfind("#[") else {
            return false;
        };
        if rest[open..].contains(needle) {
            return true;
        }
        rest = rest[..open].trim_end();
    }
    false
}

/// Directories a production scan never counts.
const NON_PRODUCTION_DIRS: &[&str] = &["target", "node_modules", "tests"];

/// The one member of [`NON_PRODUCTION_DIRS`] every walk still descends into.
const WALKED_FOR_DECLARATIONS: &str = "tests";

/// Whether a directory is pruned from **every** walk — build output and
/// vendored packages, which hold no first-party Rust source at any depth.
///
/// Derived from [`NON_PRODUCTION_DIRS`] rather than spelled again, because two
/// lists that disagree about what a walk covers is the exact failure this module
/// already carries scars from ([`production_rust_files`]). The single carve-out
/// is `tests/`, which is excluded from *counting*, not from *walking*:
/// [`cfg_test_only_files`] has to read files under `src/**/tests/` because
/// test-only-ness propagates *through* them into production-named children, so
/// pruning them would drop those `mod` declarations — the fail-open direction.
fn is_pruned_dir(name: &str) -> bool {
    NON_PRODUCTION_DIRS.contains(&name) && name != WALKED_FOR_DECLARATIONS
}

/// Whether `path`'s file name is one of the conventional test-file shapes.
fn is_test_file_name(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name == "tests.rs" || name.ends_with("_tests.rs")
}

/// **The** definition of "a production Rust file under `src`", as absolute
/// paths, sorted: the fatal walk, the conventional name/directory exclusions,
/// and the [`cfg_test_only_files`] subtraction, composed once.
///
/// Ratchets used to each spell this two-step recipe themselves, and they had
/// already drifted: `reborn_extension_host_port_inversion.rs` skipped
/// `node_modules` and `reborn_extension_manager_split.rs` did not. That is
/// exactly the failure mode a shrink-only list cannot survive — two gates
/// disagreeing about what production code *is* means one of them is measuring
/// a different tree than the row it enforces was written against. Raised by
/// @serrrfirat on #7003 (and independently on #7004).
///
/// I/O errors are **fatal**, not skipped: a scan that silently swallows a
/// failed `read_dir` walks a smaller tree and passes a shrink-only list
/// vacuously.
///
/// ⚠ Other ratchets in this directory still carry their own walkers. Adopting
/// this helper across them is tracked separately — see the module note in
/// `reborn_extension_manager_split.rs`.
#[allow(dead_code)]
pub fn production_rust_files(src: &Path) -> Vec<PathBuf> {
    let test_only = cfg_test_only_files(src);
    let mut all = Vec::new();
    all_rust_files(src, &mut all);
    let mut out: Vec<PathBuf> = all
        .into_iter()
        .filter(|path| {
            !is_test_file_name(path)
                && !test_only.contains(path)
                && path
                    .strip_prefix(src)
                    .map(|relative| {
                        !relative.components().any(|component| {
                            NON_PRODUCTION_DIRS
                                .iter()
                                .any(|skip| component.as_os_str() == *skip)
                        })
                    })
                    .unwrap_or(true)
        })
        .collect();
    out.sort();
    out
}

/// Every `.rs` file under `src` that is test-only **despite a
/// production-looking name**: reachable only through a `#[cfg(test)]`-gated
/// out-of-line `mod` declaration, or (transitively) declared from a file that
/// is itself test-only. The name-based exclusions the production walkers use
/// (`tests.rs`, `*_tests.rs`, `tests/` directories) cannot see these — e.g. a
/// `#[cfg(test)] mod e2e_tests;` whose `e2e_tests.rs` then declares
/// `mod fixture;` leaves `fixture.rs` looking like production code. A scan
/// that counts such a file can keep a residue row or an implementor pin
/// "satisfied" with a test double. I/O errors are fatal, same as the walkers.
#[allow(dead_code)]
pub fn cfg_test_only_files(src: &Path) -> BTreeSet<PathBuf> {
    let mut all = Vec::new();
    all_rust_files(src, &mut all);

    // file -> (declared module name, cfg(test)-gated) for out-of-line decls,
    // plus any `#[path = "…"]` overrides (read from raw source — the path
    // lives in a string literal the stripper blanks).
    let mut decls: BTreeMap<PathBuf, Vec<(String, bool)>> = BTreeMap::new();
    let mut path_overrides: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
    for file in &all {
        let source = std::fs::read_to_string(file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        let found = out_of_line_mod_decls(&strip_comments_and_strings(&source));
        if !found.is_empty() {
            decls.insert(file.clone(), found);
        }
        let overrides = explicit_mod_paths(&source);
        if !overrides.is_empty() {
            path_overrides.insert(file.clone(), overrides);
        }
    }

    let resolve = |declaring: &Path, name: &str| -> Option<PathBuf> {
        if let Some(explicit) = path_overrides
            .get(declaring)
            .and_then(|overrides| overrides.get(name))
        {
            // `#[path]` on an out-of-line mod resolves relative to the
            // declaring file's directory (mod.rs/lib.rs owners) or its module
            // directory (2018-style files); accept whichever exists.
            let parent = declaring.parent()?;
            return [
                parent.join(explicit),
                parent.join(declaring.file_stem()?).join(explicit),
            ]
            .into_iter()
            .find(|candidate| candidate.is_file());
        }
        let file_name = declaring.file_name().and_then(|n| n.to_str())?;
        let base = if matches!(file_name, "mod.rs" | "lib.rs" | "main.rs") {
            declaring.parent()?.to_path_buf()
        } else {
            declaring.parent()?.join(declaring.file_stem()?)
        };
        [
            base.join(format!("{name}.rs")),
            base.join(name).join("mod.rs"),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
    };

    let is_test_by_name = |path: &Path| -> bool {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        name == "tests.rs"
            || name.ends_with("_tests.rs")
            || path
                .strip_prefix(src)
                .map(|relative| {
                    relative
                        .components()
                        .any(|component| component.as_os_str() == "tests")
                })
                .unwrap_or(false)
    };

    // Seed: targets of `#[cfg(test)] mod …;` declarations, plus every file the
    // name convention already marks (so test-only-ness propagates *through*
    // them into production-named children).
    let mut test_only: BTreeSet<PathBuf> = all
        .iter()
        .filter(|file| is_test_by_name(file))
        .cloned()
        .collect();
    for (declaring, mods) in &decls {
        for (name, gated) in mods {
            if *gated && let Some(target) = resolve(declaring, name) {
                test_only.insert(target);
            }
        }
    }

    // Fixpoint: everything declared from a test-only file is test-only.
    loop {
        let mut grew = false;
        for (declaring, mods) in &decls {
            if !test_only.contains(declaring) {
                continue;
            }
            for (name, _) in mods {
                if let Some(target) = resolve(declaring, name)
                    && test_only.insert(target)
                {
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }

    // Only the surprises: the name-based ones are already excluded everywhere.
    test_only
        .into_iter()
        .filter(|file| !is_test_by_name(file))
        .collect()
}

/// Every `.rs` file under `dir`, with no test-*shape* exclusions (the caller
/// decides). Prunes only the directories [`is_pruned_dir`] names — build output
/// and vendored packages. I/O errors are fatal.
fn all_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("cannot read an entry under {}: {error}", dir.display())
        });
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_pruned_dir)
            {
                continue;
            }
            all_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Replace line comments, block comments (nested), plain/raw string literal
/// contents, and char literals with blanks, preserving newlines so the
/// line-based matcher keeps operating on real code lines only. A minimal
/// lexer — good enough for rustfmt'd source; it intentionally errs on the side
/// of stripping (a mis-lex would surface loudly as a frozen-set mismatch).
pub fn strip_comments_and_strings(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Line comment.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment (Rust block comments nest).
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 1usize;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }
        // Raw string literal: r"..." / r#"..."# (optionally b/c-prefixed).
        if c == 'r' || ((c == 'b' || c == 'c') && chars.get(i + 1) == Some(&'r')) {
            let hash_start = if c == 'r' { i + 1 } else { i + 2 };
            let mut j = hash_start;
            while chars.get(j) == Some(&'#') {
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                let hashes = j - hash_start;
                let mut k = j + 1;
                while k < chars.len() {
                    if chars[k] == '"' && (0..hashes).all(|h| chars.get(k + 1 + h) == Some(&'#')) {
                        k += 1 + hashes;
                        break;
                    }
                    if chars[k] == '\n' {
                        out.push('\n');
                    }
                    k += 1;
                }
                i = k;
                continue;
            }
        }
        // Plain string literal (handles escapes).
        if c == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                if chars[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        // Char literal vs lifetime: only consume when it closes as a literal.
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\\') {
                let mut k = i + 2;
                while k < chars.len() && chars[k] != '\'' {
                    k += 1;
                }
                i = k + 1;
                continue;
            }
            if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
            // A lifetime (`'a`) — emit and move on.
            out.push(c);
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}
