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
//! - a **second definition** of a frozen name (same file or another
//!   module/crate) fails — occurrences are preserved and multiplicity is checked
//!   explicitly, so duplicate-name debt cannot hide behind an existing entry;
//! - **deleting** a store without removing it from [`FROZEN_INMEMORY_STORES`]
//!   also fails — so the allowlist is forced to shrink in lock-step as each
//!   domain lands, and a reviewer sees the list get shorter (§10: "compare set
//!   membership, never an aggregate count").
//!
//! Scanner semantics (shared with the other §10 ratchets — see
//! [`ratchet_support`]): comments/strings stripped before matching; skips
//! `tests/`, `examples/`, and `benches/` trees; line-based, not cfg-aware — a
//! pub-visible store in an inline `#[cfg(test)]` module in src IS inventoried,
//! so keep test doubles under `tests/` (or justify an allowlist entry in
//! review).
//!
//! Definition of done for this axis (§10): the allowlist reaches the empty set —
//! every store is `Filesystem*Store<InMemoryBackend>` in tests. Until then this
//! frozen set is the contract.

mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, duplicate_definitions, scan_type_defs, workspace_root,
};

const KEYWORDS: &[&str] = &["struct "];

fn is_inmemory_store(ident: &str) -> bool {
    ident.starts_with("InMemory") && ident.ends_with("Store")
}

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
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crates_dir,
        KEYWORDS,
        &is_inmemory_store,
        &[
            "reborn_inmemory_store_ratchet.rs",
            "reborn_localdev_typename_ratchet.rs",
        ],
        &mut found,
    );

    let frozen: BTreeSet<&str> = FROZEN_INMEMORY_STORES.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.keys().map(String::as_str).collect();

    let added: Vec<(&str, &Vec<TypeDefOccurrence>)> = found
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

/// Self-test for the shared scanner as this ratchet configures it: it must
/// extract exactly the pub-visible (including `pub(crate)`/`pub(super)`/
/// `pub(in ...)`) `struct InMemory*Store` definitions and ignore private
/// structs, non-`Store` structs, and — because comments and strings are
/// stripped before matching — definition-shaped text in line comments, block
/// comments (nested and multiline), plain string literals, and raw string
/// literals.
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
    let got: Vec<String> = scan_type_defs(sample, KEYWORDS, &is_inmemory_store)
        .into_iter()
        .map(|(ident, _)| ident)
        .collect();
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
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    for path in ["crate_a/src/lib.rs", "crate_b/src/lib.rs"] {
        for (ident, cfg_gated) in
            scan_type_defs("pub struct InMemoryDupStore;", KEYWORDS, &is_inmemory_store)
        {
            found.entry(ident).or_default().push(TypeDefOccurrence {
                path: PathBuf::from(path),
                cfg_gated,
            });
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
    let occurrences = scan_type_defs(sample, KEYWORDS, &is_inmemory_store);
    let idents: Vec<&str> = occurrences
        .iter()
        .map(|(ident, _)| ident.as_str())
        .collect();
    assert_eq!(
        idents,
        vec!["InMemoryDupStore", "InMemoryDupStore"],
        "same-file duplicates must be preserved by the scan"
    );

    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    for (ident, cfg_gated) in occurrences {
        found.entry(ident).or_default().push(TypeDefOccurrence {
            path: PathBuf::from("crate_a/src/lib.rs"),
            cfg_gated,
        });
    }
    let duplicated = duplicate_definitions(&found);
    assert_eq!(
        duplicated.len(),
        1,
        "a same-file duplicate must be flagged by the multiplicity check"
    );
    assert_eq!(duplicated[0].1.len(), 2);
}
