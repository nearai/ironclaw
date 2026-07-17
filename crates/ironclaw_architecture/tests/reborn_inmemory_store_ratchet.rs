//! Anti-slippage ratchet for the store-consolidation axis (§10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §4.3 replaces every hand-written `InMemory*Store` with the one production
//! `Filesystem*Store<F>` exercised over `InMemoryBackend`. That migration is
//! incremental (per domain), so this test **freezes the current inventory of
//! pub-visible `struct InMemory*Store` definitions** and fails on any change:
//!
//! - a **new** `InMemory*Store` (not in the allowlist) fails — the debt can only
//!   shrink, never grow;
//! - a **second definition** of a frozen name (in another module/crate) fails —
//!   the set is keyed by identifier, so multiplicity is checked explicitly and
//!   duplicate-name debt cannot hide behind an existing entry;
//! - **deleting** a store without removing it from [`FROZEN_INMEMORY_STORES`]
//!   also fails — so the allowlist is forced to shrink in lock-step as each
//!   domain lands, and a reviewer sees the list get shorter (§10: "compare set
//!   membership, never an aggregate count").
//!
//! The scanner strips comments and string literals before matching. It skips
//! `tests/`, `examples/`, and `benches/` trees (test doubles there are not §4.3
//! debt) but is line-based, not cfg-aware: a pub-visible store defined in an
//! inline `#[cfg(test)]` module in src IS inventoried — keep test doubles under
//! `tests/` (or justify an allowlist entry in review).
//!
//! Definition of done for this axis (§10): the allowlist reaches the empty set —
//! every store is `Filesystem*Store<InMemoryBackend>` in tests. Until then this
//! frozen set is the contract.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The frozen inventory of pub-visible `struct InMemory*Store` definitions under
/// `crates/`, as of the run-state slice (§4.3, A4). Remove an entry in the same
/// PR that deletes its store; never add one. Grouped by whether it is a
/// remaining Slice-A consolidation target or a peripheral store outside the
/// domain-store scope of §4.3 (the axis is still complete only when the whole
/// list is empty).
const FROZEN_INMEMORY_STORES: &[&str] = &[
    // --- remaining Slice-A domain store: turns (do last, reconcile-then-delete;
    //     `InMemoryTurnStateStore` is also the `inmemory-turn-state` production
    //     runtime authority, so it is not a mechanical delete) ---
    "InMemoryTurnStateStore",
    "InMemoryCheckpointStateStore",
    "InMemoryLoopCheckpointStore",
    "InMemoryInstructionMaterializationStore",
    // --- peripheral stores (outside §4.3's five core domains; listed so the
    //     ratchet stays exhaustive and no new InMemory store slips in) ---
    "InMemoryBoundedSubagentGoalStore",
    "InMemoryBudgetGateStore",
    "InMemoryDeliveredGateRouteStore",
    "InMemoryExtensionInstallationStore",
    "InMemoryOpenAiCompatRefStore",
    "InMemoryOutboundStateStore",
    "InMemorySecretStore",
    "InMemorySessionStore",
    "InMemoryTriggeredRunDeliveryStore",
    // --- pub(crate) stores the visibility-aware scanner also inventories
    //     (same debt class, just crate-private) ---
    "InMemorySecretsStore",
    "InMemorySlackChannelRouteStore",
    "InMemorySlackPersonalDmTargetStore",
];

#[test]
fn reborn_inmemory_store_allowlist_is_frozen_and_only_shrinks() {
    let crates_dir = workspace_root().join("crates");
    let mut found: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    collect_inmemory_store_defs(&crates_dir, &mut found);

    let frozen: BTreeSet<&str> = FROZEN_INMEMORY_STORES.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.keys().map(String::as_str).collect();

    let added: Vec<(&str, &Vec<PathBuf>)> = found
        .iter()
        .filter(|(name, _)| !frozen.contains(name.as_str()))
        .map(|(name, paths)| (name.as_str(), paths))
        .collect();
    assert!(
        added.is_empty(),
        "New pub-visible `struct InMemory*Store` definitions are banned \
         (arch-simplification §4.3/§10): every domain store must be \
         `Filesystem*Store<InMemoryBackend>`. Offending new stores: {added:?}. If this \
         is a genuine new local store, justify it in review and add it to \
         FROZEN_INMEMORY_STORES — but the intended direction is to delete these, not \
         add them."
    );

    let duplicated = duplicate_definitions(&found);
    assert!(
        duplicated.is_empty(),
        "Each frozen InMemory*Store name must have exactly one definition; a second \
         same-named definition elsewhere is new debt hiding behind an allowlist entry \
         (§10): {duplicated:?}"
    );

    let removed: Vec<&&str> = frozen.difference(&found_refs).collect();
    assert!(
        removed.is_empty(),
        "FROZEN_INMEMORY_STORES lists stores that no longer exist: {removed:?}. A store \
         was deleted (good!) — trim it from the allowlist in the same PR so the ratchet \
         keeps shrinking toward empty (§10)."
    );
}

/// Self-test for the scanner: it must extract exactly the pub-visible
/// (including `pub(crate)`/`pub(super)`/`pub(in ...)`) `struct InMemory*Store`
/// definitions and ignore private structs, non-`Store` structs, and — because
/// comments and strings are stripped before matching — definition-shaped text in
/// line comments, block comments (nested and multiline), plain string literals,
/// and raw string literals.
#[test]
fn inmemory_store_def_scanner_self_test() {
    let sample = r##"
        pub struct InMemoryWidgetStore { field: u8 }
        struct InMemoryPrivateStore;            // not pub-visible -> ignored
        pub struct InMemoryWidget { x: u8 }     // not a *Store -> ignored
        // pub struct InMemoryLineCommentedStore -> ignored
        /* pub struct InMemoryBlockCommentedStore */
        /*
        pub struct InMemoryMultilineCommentedStore { x: u8 }
        /* pub struct InMemoryNestedCommentedStore */
        */
        let name = "pub struct InMemoryStringLiteralStore";
        let raw = r#"
        pub struct InMemoryRawStringStore;
        "#;
        pub struct   InMemorySpacedStore(u8);   // extra spaces tolerated
        pub(crate) struct InMemoryCrateStore;   // restricted visibility -> still inventoried
        pub(in crate::foo) struct InMemoryInPathStore; // restricted visibility -> still inventoried
        pub(crate)fn not_a_struct() {}          // no `struct` after visibility -> ignored
        #[cfg(test)]
        mod tests {
            // The scanner is line-based, not cfg-aware: an inline cfg(test)
            // double in src IS inventoried (keep doubles under `tests/`).
            pub struct InMemoryCfgTestStore;
        }
    "##;
    let got = scan_source_for_inmemory_store_defs(sample);
    assert_eq!(
        got,
        vec![
            "InMemoryWidgetStore",
            "InMemorySpacedStore",
            "InMemoryCrateStore",
            "InMemoryInPathStore",
            "InMemoryCfgTestStore"
        ],
        "scanner must match pub-visible `struct InMemory*Store` definitions \
         outside comments and strings, in source order"
    );
}

/// Self-test for the multiplicity check: the same identifier defined in two
/// files must be reported as a duplicate.
#[test]
fn inmemory_store_duplicate_detection_self_test() {
    let mut found: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in ["crate_a/src/lib.rs", "crate_b/src/lib.rs"] {
        for ident in scan_source_for_inmemory_store_defs("pub struct InMemoryDupStore;") {
            found.entry(ident).or_default().push(PathBuf::from(path));
        }
    }
    let duplicated = duplicate_definitions(&found);
    assert_eq!(
        duplicated.len(),
        1,
        "two same-named definitions must be flagged"
    );
    assert_eq!(duplicated[0].0, "InMemoryDupStore");
    assert_eq!(duplicated[0].1.len(), 2);
}

/// Self-test for same-file multiplicity: two same-named definitions in
/// different modules of ONE file must also be reported — the scan preserves
/// occurrences instead of deduplicating per file.
#[test]
fn inmemory_store_same_file_duplicate_detection_self_test() {
    let sample = r#"
        mod first {
            pub struct InMemoryDupStore;
        }
        mod second {
            pub struct InMemoryDupStore;
        }
    "#;
    let occurrences = scan_source_for_inmemory_store_defs(sample);
    assert_eq!(
        occurrences,
        vec!["InMemoryDupStore", "InMemoryDupStore"],
        "same-file duplicates must be preserved by the scan"
    );

    let mut found: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for ident in occurrences {
        found
            .entry(ident)
            .or_default()
            .push(PathBuf::from("crate_a/src/lib.rs"));
    }
    let duplicated = duplicate_definitions(&found);
    assert_eq!(
        duplicated.len(),
        1,
        "a same-file duplicate must be flagged by the multiplicity check"
    );
    assert_eq!(duplicated[0].1.len(), 2);
}

fn duplicate_definitions(found: &BTreeMap<String, Vec<PathBuf>>) -> Vec<(&str, &Vec<PathBuf>)> {
    found
        .iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(name, paths)| (name.as_str(), paths))
        .collect()
}

fn collect_inmemory_store_defs(dir: &Path, out: &mut BTreeMap<String, Vec<PathBuf>>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts and non-production trees: doubles defined
            // under `tests/` (e.g. recording stores in tests/support/),
            // `examples/`, or `benches/` are not the §4.3 production-store debt
            // this ratchet inventories. NOTE: an inline `#[cfg(test)]` module in
            // src IS still scanned — the scanner is line-based, not cfg-aware —
            // so keep test doubles under `tests/` (or justify an allowlist
            // entry in review).
            let dir_name = path.file_name().and_then(|n| n.to_str());
            if matches!(dir_name, Some("target" | "tests" | "examples" | "benches")) {
                continue;
            }
            collect_inmemory_store_defs(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        // This ratchet file's own allowlist and self-test fixtures mention
        // `InMemory*Store` names; string/comment stripping already excludes
        // them, but skip the file entirely as defense in depth.
        if path.file_name().and_then(|n| n.to_str()) == Some("reborn_inmemory_store_ratchet.rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for ident in scan_source_for_inmemory_store_defs(&contents) {
            out.entry(ident).or_default().push(path.clone());
        }
    }
}

/// Extract the identifier from every pub-visible `struct InMemory*Store`
/// definition — `pub`, `pub(crate)`, `pub(super)`, or `pub(in path)` (a
/// restricted-visibility store is the same debt class, just crate-private).
/// Comments and string literals are stripped first, so definition-shaped text
/// inside them is not matched. Matches the definition form, not references.
/// Returns every occurrence in source order (no dedup) so same-file duplicate
/// definitions in different modules stay visible to the multiplicity check.
fn scan_source_for_inmemory_store_defs(source: &str) -> Vec<String> {
    let stripped = strip_comments_and_strings(source);
    let mut out = Vec::new();
    for line in stripped.lines() {
        let trimmed = line.trim_start();
        let Some(after_pub) = trimmed.strip_prefix("pub") else {
            continue;
        };
        // Optional restricted-visibility qualifier: `(crate)`, `(super)`, `(in path)`.
        let after_vis = match after_pub.trim_start().strip_prefix('(') {
            Some(rest) => match rest.split_once(')') {
                Some((_, tail)) => tail,
                None => continue,
            },
            None => after_pub,
        };
        let Some(rest) = after_vis.trim_start().strip_prefix("struct ") else {
            continue;
        };
        let ident: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.starts_with("InMemory") && ident.ends_with("Store") {
            out.push(ident);
        }
    }
    out
}

/// Replace line comments, block comments (nested), plain/raw string literal
/// contents, and char literals with blanks, preserving newlines so the
/// line-based matcher keeps operating on real code lines only. A minimal
/// lexer — good enough for rustfmt'd source; it intentionally errs on the side
/// of stripping (a mis-lex would surface loudly as a frozen-set mismatch).
fn strip_comments_and_strings(source: &str) -> String {
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}
