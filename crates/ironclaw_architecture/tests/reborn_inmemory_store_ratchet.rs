//! Anti-slippage ratchet for the store-consolidation axis (§10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §4.3 replaces every hand-written `InMemory*Store` with the one production
//! `Filesystem*Store<F>` exercised over `InMemoryBackend`. That migration is
//! incremental (per domain), so this test **freezes the current inventory of
//! `pub struct InMemory*Store` definitions** and fails on any change:
//!
//! - a **new** `InMemory*Store` (not in the allowlist) fails — the debt can only
//!   shrink, never grow;
//! - **deleting** a store without removing it from [`FROZEN_INMEMORY_STORES`]
//!   also fails — so the allowlist is forced to shrink in lock-step as each
//!   domain lands, and a reviewer sees the list get shorter (§10: "compare set
//!   membership, never an aggregate count").
//!
//! Definition of done for this axis (§10): the allowlist reaches the empty set —
//! every store is `Filesystem*Store<InMemoryBackend>` in tests. Until then this
//! frozen set is the contract.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The frozen inventory of `pub struct InMemory*Store` definitions under
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
];

#[test]
fn reborn_inmemory_store_allowlist_is_frozen_and_only_shrinks() {
    let crates_dir = workspace_root().join("crates");
    let mut found = BTreeSet::new();
    collect_inmemory_store_defs(&crates_dir, &mut found);

    let frozen: BTreeSet<&str> = FROZEN_INMEMORY_STORES.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.iter().map(String::as_str).collect();

    let added: Vec<&&str> = found_refs.difference(&frozen).collect();
    assert!(
        added.is_empty(),
        "New `pub struct InMemory*Store` definitions are banned (arch-simplification \
         §4.3/§10): every domain store must be `Filesystem*Store<InMemoryBackend>`. \
         Offending new stores: {added:?}. If this is a genuine new local store, justify \
         it in review and add it to FROZEN_INMEMORY_STORES — but the intended direction \
         is to delete these, not add them."
    );

    let removed: Vec<&&str> = frozen.difference(&found_refs).collect();
    assert!(
        removed.is_empty(),
        "FROZEN_INMEMORY_STORES lists stores that no longer exist: {removed:?}. A store \
         was deleted (good!) — trim it from the allowlist in the same PR so the ratchet \
         keeps shrinking toward empty (§10)."
    );
}

/// Self-test for the scanner: it must extract exactly the `pub struct InMemory*Store`
/// definitions and ignore string literals, comments, and non-`Store` structs.
#[test]
fn inmemory_store_def_scanner_self_test() {
    let sample = r#"
        pub struct InMemoryWidgetStore { field: u8 }
        struct InMemoryPrivateStore;            // not `pub` -> ignored
        pub struct InMemoryWidget { x: u8 }     // not a *Store -> ignored
        // pub struct InMemoryCommentedStore    -> in a comment, but scanner is line-based
        let name = "InMemoryStringLiteralStore"; // string literal, no `pub struct` -> ignored
        pub struct   InMemorySpacedStore(u8);   // extra spaces tolerated
    "#;
    let mut out = BTreeSet::new();
    scan_source_for_inmemory_store_defs(sample, &mut out);
    let got: Vec<&str> = out.iter().map(String::as_str).collect();
    assert_eq!(
        got,
        vec!["InMemorySpacedStore", "InMemoryWidgetStore"],
        "scanner must match only `pub struct InMemory*Store` definitions"
    );
}

fn collect_inmemory_store_defs(dir: &Path, out: &mut BTreeSet<String>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts.
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_inmemory_store_defs(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        // This ratchet file's own self-test fixture contains sample
        // `pub struct InMemory*Store` lines; don't count them.
        if path.file_name().and_then(|n| n.to_str()) == Some("reborn_inmemory_store_ratchet.rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        scan_source_for_inmemory_store_defs(&contents, out);
    }
}

/// Line-based scan: pull the identifier from any `pub struct InMemory*Store`
/// definition. Intentionally simple (no regex dep); matches the definition form,
/// not references, string literals, or comments.
fn scan_source_for_inmemory_store_defs(source: &str, out: &mut BTreeSet<String>) {
    const MARKER: &str = "pub struct ";
    for line in source.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix(MARKER) else {
            continue;
        };
        let ident: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.starts_with("InMemory") && ident.ends_with("Store") {
            out.insert(ident);
        }
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}
