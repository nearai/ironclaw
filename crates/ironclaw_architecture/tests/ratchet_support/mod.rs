//! Shared scanner machinery for the §10 anti-slippage ratchets
//! (`reborn_inmemory_store_ratchet.rs`, `reborn_localdev_typename_ratchet.rs`).
//!
//! One hardened implementation of the walk/strip/match pipeline, so every
//! ratchet gets the same guarantees: comment/string stripping, restricted
//! visibility (`pub(crate)`/`pub(super)`/`pub(in …)`), optional `unsafe`/`auto`
//! modifiers, occurrence-preserving scans (same-file duplicates stay visible),
//! and a production-scoped walk (skips `target/`, `tests/`, `examples/`,
//! `benches/`). The scanners are line-based, not cfg-aware: a pub-visible
//! definition in an inline `#[cfg(test)]` module in src IS inventoried — keep
//! test doubles under `tests/` (or justify an allowlist entry in review).

use std::collections::BTreeMap;
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

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}

/// Names with more than one defining occurrence — a second same-named
/// definition elsewhere is new debt hiding behind an allowlist entry (§10) —
/// EXCEPT when every occurrence is `#[cfg(...)]`-gated (mutually exclusive
/// compile branches of the same type, the factory durable/no-durable pattern).
/// A mix of gated and ungated occurrences is still flagged.
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
