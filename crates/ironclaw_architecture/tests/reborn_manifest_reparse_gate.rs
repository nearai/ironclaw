//! REC-1 gate: raw manifest TOML is reparsed only by the **compiler** and
//! **bundled-asset** paths — never by a projection/loader
//! path that reads an installed extension. An installed extension projects
//! from its persisted resolved record (`ExtensionManifestRecord::from_resolved`
//! / `resolved.to_internal`), so a NEW `ExtensionManifest::parse` /
//! `ExtensionManifestRecord::from_toml` call — especially in a projection path
//! such as the WebUI services projection or the generic host loader — fails
//! this gate until its file and category are enumerated below.
//!
//! Pattern mirrors `reborn_extension_specificity.rs`: scan production Rust with
//! test files and `#[cfg(test)]` blocks stripped, collect the reparse call
//! sites, and hold them to an EXACT, categorized allowlist. Adding a reparse
//! (new file, or one more call in an existing file) fails until the allowlist
//! is updated with a justification; removing one — moving a projection onto the
//! resolved record — fails the stale entry. The list can only shrink.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates/")
        .to_path_buf()
}

/// Why a reparse site is *not* a projection reparse. Every allowlisted entry
/// names one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReparseCategory {
    /// One-time compile of a manifest into a resolved record at install/ingest.
    Compiler,
    /// Compile of a host-bundled static manifest asset — there is no installed
    /// resolved record for a pure bundled descriptor.
    BundledAsset,
}

/// `(path, call_count, category, justification)`. Every production reparse site
/// lives here; the call count is pinned so a new reparse in an existing
/// compiler module still forces a conscious allowlist edit.
const ALLOWLIST: &[(&str, usize, ReparseCategory, &str)] = &[
    (
        "crates/ironclaw_extensions/src/lib.rs",
        1,
        ReparseCategory::Compiler,
        "the canonical manifest-file loader/parser in the manifest-owning crate (load_package_entry)",
    ),
    (
        "crates/ironclaw_extensions/src/installations.rs",
        1,
        ReparseCategory::Compiler,
        "one-time CAS migration compiles pre-resolved filesystem rows before the store opens",
    ),
    (
        "crates/ironclaw_product/src/adapter_registry.rs",
        1,
        ReparseCategory::Compiler,
        "parse_product_adapter_manifest_record — the registry manifest compiler entry",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs",
        3,
        ReparseCategory::BundledAsset,
        "bundled first-party package + filesystem-root catalog compile",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs",
        3,
        ReparseCategory::Compiler,
        "install/activation-time compile of the installed manifest into its resolved record",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/available_extension_import.rs",
        1,
        ReparseCategory::Compiler,
        "install-time compile of an imported (zip-uploaded) manifest into its resolved record",
    ),
    (
        "crates/ironclaw_host_runtime/src/memory_native_extension.rs",
        1,
        ReparseCategory::BundledAsset,
        "native_memory_first_party_package — compiles the bundled ironclaw.memory manifest asset (include_str! of assets/memory_native/manifest.toml); a host-bundled descriptor has no installed resolved record to project from",
    ),
];

fn is_test_source_path(path: &Path) -> bool {
    let mut components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string());
    if components.any(|component| {
        component == "tests"
            || component == "__tests__"
            || component == "test-utils"
            || component == "test_support"
    }) {
        return true;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    name == "tests.rs"
        || name == "test_support.rs"
        || name.ends_with("_tests.rs")
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// Strip `#[cfg(test)]` items (inline `mod tests { … }` blocks and `mod tests;`
/// declarations) before matching — the same brace-counting heuristic
/// `reborn_extension_specificity.rs` uses.
fn strip_cfg_test_blocks(source: &str) -> String {
    let mut kept = String::with_capacity(source.len());
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with("#[cfg(test)]") {
            kept.push_str(line);
            kept.push('\n');
            continue;
        }
        let mut depth: i64 = 0;
        let mut opened = false;
        for skipped in lines.by_ref() {
            let trimmed = skipped.trim_start();
            if !opened && trimmed.starts_with("#[") {
                continue;
            }
            depth += skipped.matches('{').count() as i64;
            depth -= skipped.matches('}').count() as i64;
            if !opened {
                if skipped.contains('{') {
                    opened = true;
                } else if trimmed.ends_with(';') {
                    break;
                }
            }
            if opened && depth <= 0 {
                break;
            }
        }
    }
    kept
}

/// Count reparse calls in a production source file, ignoring comment lines so a
/// doc-comment mention of the API is not a false positive.
fn count_reparse_calls(source: &str) -> usize {
    let stripped = strip_cfg_test_blocks(source);
    stripped
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with('*')
        })
        .map(|line| {
            line.matches("ExtensionManifest::parse(").count()
                + line.matches("ExtensionManifestRecord::from_toml(").count()
        })
        .sum()
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            if matches!(
                name.as_deref(),
                Some("target") | Some(".git") | Some("node_modules")
            ) {
                continue;
            }
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn manifest_reparse_stays_within_the_compiler_and_bundled_paths() {
    let root = workspace_root();
    let mut files = Vec::new();
    for subtree in ["crates", "src"] {
        collect_rust_files(&root.join(subtree), &mut files);
    }

    // Production (test-stripped) reparse counts, keyed by workspace-relative path.
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for path in &files {
        if is_test_source_path(path) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let count = count_reparse_calls(&source);
        if count > 0 {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            found.insert(rel, count);
        }
    }

    let allow: BTreeMap<String, usize> = ALLOWLIST
        .iter()
        .map(|(path, count, _, _)| ((*path).to_string(), *count))
        .collect();

    let mut problems = Vec::new();
    for (path, count) in &found {
        match allow.get(path) {
            None => problems.push(format!(
                "NEW manifest reparse in `{path}` ({count} call(s)) — a projection/loader path must \
                 read the persisted resolved record (from_resolved / resolved.to_internal), not \
                 reparse raw TOML. If this is a compiler/bundled-asset site, add it to \
                 ALLOWLIST with its category and justification."
            )),
            Some(expected) if expected != count => problems.push(format!(
                "manifest reparse count changed in `{path}`: allowlist says {expected}, found \
                 {count}. Update the ALLOWLIST count and confirm the new call is not a projection \
                 reparse."
            )),
            Some(_) => {}
        }
    }
    for path in allow.keys() {
        if !found.contains_key(path) {
            problems.push(format!(
                "STALE allowlist entry `{path}` — no manifest reparse found there anymore. Remove \
                 the entry (the list is shrink-only)."
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "REC-1 manifest-reparse gate:\n  - {}",
        problems.join("\n  - ")
    );
}
