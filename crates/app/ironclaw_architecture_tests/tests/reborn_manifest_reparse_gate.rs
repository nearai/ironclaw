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

#[allow(dead_code)]
mod ratchet_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ratchet_support::{resolve_crate_relative, strip_line_anchored_cfg_test_items, workspace_root};

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
        "crates/ironclaw_extension_registry/src/lib.rs",
        1,
        ReparseCategory::Compiler,
        "the canonical manifest-file loader/parser in the manifest-owning crate (load_package_entry)",
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

/// Count reparse calls in a production source file, ignoring comment lines so a
/// doc-comment mention of the API is not a false positive.
fn count_reparse_calls(source: &str) -> usize {
    let stripped = strip_line_anchored_cfg_test_items(source);
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

/// Walk `dir` for `.rs` files, skipping build output, `.git`, and vendored
/// frontend packages.
///
/// An unreadable directory or entry is an error, not a silently skipped
/// subtree: the `files.len() >= 500` floor only detects a scan that went almost
/// entirely dark, so one unreadable crate tree would leave the count healthy
/// while the gate reported zero reparse sites for files it never opened
/// (CHECKLIST WS0, #6963).
fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| read_failure(dir, error))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to read an entry in {}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            if matches!(
                name.as_deref(),
                Some("target") | Some(".git") | Some("node_modules")
            ) {
                continue;
            }
            collect_rust_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn read_failure(path: &Path, error: std::io::Error) -> String {
    format!(
        "failed to read {} — an unreadable path must fail this gate, not vanish from the \
         scan: {error}",
        path.display()
    )
}

/// The walk refuses an unreadable path rather than shrinking around it. The
/// floor cannot cover this: one unreadable crate tree leaves the file count far
/// above 500 while the reparse census silently loses everything under it.
#[test]
fn an_unreadable_path_fails_the_walk_rather_than_disappearing_from_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let not_a_directory = temp.path().join("regular-file");
    std::fs::write(&not_a_directory, "// fixture\n").expect("fixture file");

    let mut files = Vec::new();
    let error = collect_rust_files(&not_a_directory, &mut files)
        .expect_err("an unreadable directory must refuse");
    assert!(
        error.contains("must fail this gate, not vanish from the scan"),
        "expected an I/O refusal naming the path, got: {error}"
    );

    // Generated trees stay excluded, so a `pnpm install` cannot inflate the
    // scan with vendored sources.
    for generated in ["target", "node_modules", ".git"] {
        let vendored = temp.path().join(format!("tree/{generated}/pkg"));
        std::fs::create_dir_all(&vendored).expect("vendored tree");
        std::fs::write(vendored.join("lib.rs"), "// vendored\n").expect("vendored source");
    }
    std::fs::write(temp.path().join("tree/real.rs"), "// real\n").expect("real source");
    let mut files = Vec::new();
    collect_rust_files(&temp.path().join("tree"), &mut files).expect("a readable tree walks");
    assert_eq!(
        files,
        vec![temp.path().join("tree/real.rs")],
        "the walk must skip generated trees and keep everything else"
    );
}

#[test]
fn manifest_reparse_stays_within_the_compiler_and_bundled_paths() {
    let root = workspace_root();
    let mut files = Vec::new();
    // The scan used to cover `["crates", "src"]`; `src` was the v1 monolith and
    // is long gone. `collect_rust_files` skips a missing directory silently, so
    // the dead entry was invisible — and the same silence is what a wrong root
    // produces. The scan root must exist (CHECKLIST WS0, #6963).
    let crates_dir = root.join("crates");
    assert!(
        crates_dir.is_dir(),
        "manifest-reparse scan root {} does not exist — repoint it rather than \
         scanning nothing",
        crates_dir.display()
    );
    collect_rust_files(&crates_dir, &mut files)
        .expect("the manifest-reparse walk must not hit an I/O error");
    assert!(
        files.len() >= 500,
        "manifest-reparse scan collected only {} file(s) — the walk is broken, not the \
         workspace; an empty scan reports zero reparse sites and passes.",
        files.len()
    );

    // Production (test-stripped) reparse counts, keyed by workspace-relative path.
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for path in &files {
        if is_test_source_path(path) {
            continue;
        }
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}", read_failure(path, error)));
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

    // ALLOWLIST paths keep their readable flat `crates/<crate>/…` spelling and
    // are RESOLVED through the crate inventory before being matched against a
    // scanned file. Without that, the family move (PROPOSAL §5) makes every
    // entry stale AND every real site look new (CHECKLIST WS10).
    let allow: BTreeMap<String, usize> = ALLOWLIST
        .iter()
        .map(|(path, count, _, _)| (resolve_crate_relative(&root, path), *count))
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
