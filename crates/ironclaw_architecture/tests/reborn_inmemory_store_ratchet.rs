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
//! - **deleting** a store without removing it from its frozen list also
//!   fails — so the allowlist is forced to shrink in lock-step as each domain
//!   lands, and a reviewer sees the list get shorter (§10: "compare set
//!   membership, never an aggregate count").
//!
//! Scanner semantics (shared with the other §10 ratchets — see
//! [`ratchet_support`]): comments/strings stripped before matching; skips
//! `tests/`, `examples/`, and `benches/` trees; line-based, not cfg-aware — a
//! pub-visible store in an inline `#[cfg(test)]` module in src IS inventoried,
//! so keep test doubles under `tests/` (or justify an allowlist entry in
//! review).
//!
//! The frozen inventory is split into two disjoint lists:
//! [`FROZEN_DEBT_INMEMORY_STORES`] (parallel store implementations §4.3 still
//! owes a consolidation) and [`JUSTIFIED_KEEP_INMEMORY_STORES`] (audited
//! keeps — each entry's annotation states why it is NOT a §1.4 lock-step
//! duplicate and must not be blindly "consolidated"). **Definition of done for
//! this axis (§10): the DEBT list shrinks to empty — and as of #6263 it IS
//! empty.** Every hand-written `InMemory*Store` on the store-consolidation axis
//! is now either deleted (consolidated onto the one production
//! `Filesystem*Store<F>` over an `InMemoryBackend`) or a justified keep. The
//! mechanical consolidations (A1–A8: approvals, authorization, processes,
//! run-state, budget-gate, the outbound family), the checkpoint cluster
//! (checkpoint-state payloads, loop-checkpoint metadata), the Slack host-state
//! test doubles, the secrets cluster (`FilesystemSecretStore::ephemeral()`),
//! and finally turns — `InMemoryTurnStateStore` collapsed into the crate-private
//! `TurnStateEngine` embedded in `FilesystemTurnStateRowStore` — are all done.
//! `FROZEN_DEBT_INMEMORY_STORES` is therefore empty; it must never grow again.

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

/// Remaining §4.3 consolidation DEBT: pub-visible `struct InMemory*Store`
/// definitions under `crates/` that duplicate (or stand in for) a durable
/// store implementation and still owe a consolidation. Remove an entry in the
/// same PR that deletes its store; never add one. Comments are stripped by the
/// scanner, so the per-entry status notes are documentation only — the
/// enforced contract is the string set.
///
/// **Status (re-assessed 2026-07-20): the §4.3 store-consolidation axis debt is
/// now ZERO.** Every hand-written `InMemory*Store` on this axis has been either
/// deleted (consolidated onto the one production `Filesystem*Store<F>` over an
/// `InMemoryBackend`) or moved to [`JUSTIFIED_KEEP_INMEMORY_STORES`] with an
/// audited reason. The last entry — `InMemoryTurnStateStore`, the
/// `inmemory-turn-state` runtime authority — was eliminated in #6263: its
/// semantics engine is now a crate-private `TurnStateEngine` embedded inside
/// `FilesystemTurnStateRowStore`, so there is exactly one public turn-state
/// store. This list is therefore empty; it must never grow (a new
/// `InMemory*Store` fails the scan below).
const FROZEN_DEBT_INMEMORY_STORES: &[&str] = &[
    // (empty — §4.3 store-consolidation debt is zero; see the doc comment above.)
    //
    // Historical, for provenance: `InMemorySecretStore`/`InMemorySecretsStore`
    // (secrets cluster) were consolidated onto `FilesystemSecretStore::ephemeral()`;
    // `InMemoryTurnStateStore` (turns, the trickiest/last case) was collapsed into
    // the private `TurnStateEngine` inside `FilesystemTurnStateRowStore` (#6263),
    // with the crash-consistency reference-model oracle
    // (`row_store_crash_consistency.rs`) now driven by a never-crashed row store.
];

/// Audited JUSTIFIED KEEPS: pub-visible `InMemory*Store` types that are NOT
/// §1.4 lock-step store duplicates. Each annotation states the reason; do not
/// "consolidate" these without first invalidating that reason in review.
const JUSTIFIED_KEEP_INMEMORY_STORES: &[&str] = &[
    // --- Bounded volatile caches next to already-wired durable stores
    //     (`FilesystemSubagentGoalStore` in the libSQL/Postgres runner
    //     adapters; `FilesystemOpenAiCompatRefStore` in OpenAI-compatible
    //     serving). Do NOT swap them for a durable store in tests that
    //     exercise the bounded/evicting cache semantics. ---
    //   BoundedSubagentGoal: capacity-bounded, evict-oldest (VecDeque insertion
    //     order) cache of in-flight subagent-spawn goals — goal_store.rs.
    "InMemoryBoundedSubagentGoalStore",
    //   OpenAiCompatRef: capacity-bounded with oldest-created eviction (evicts
    //     the minimum `created_at`; reads do not refresh recency — NOT an LRU)
    //     AND the crate's documented filesystem-free default so contract-only
    //     consumers pull no `ironclaw_filesystem` dep (openai_compat CLAUDE.md).
    "InMemoryOpenAiCompatRefStore",
    // --- Ephemeral per-run staging BY DESIGN. Constructed per claimed run /
    //     model call (`ironclaw_runner` loop_driver_host + model_gateway) to
    //     stage raw model-visible prompt content between the prompt and model
    //     ports; the run_profile contract requires raw prompt text to stay
    //     behind host implementations, so a durable variant must not exist. ---
    "InMemoryInstructionMaterializationStore",
    // --- Embedded ENGINE, not a parallel implementation.
    //     `FilesystemExtensionInstallationStore` (reborn_composition) wraps
    //     this store as its in-memory working set and adds snapshot
    //     persistence — the domain logic exists exactly once, here. Follow-up
    //     that would let it go crate-private: relocate the filesystem store
    //     down to `ironclaw_extensions` by inverting its manifest-decoder deps
    //     (`product_extension_host_api_contract_registry`,
    //     `default_host_port_catalog`) into a constructor parameter. ---
    "InMemoryExtensionInstallationStore",
    // --- Dev/test-only by explicit feature gate (`test-support`).
    //     The production counterpart already exists and is restart-safe by
    //     being STATELESS: `SignedTokenSessionStore` (HMAC-signed bearers,
    //     signed_session_login.rs); durable revocation is an optional
    //     DB-backed `SessionStore` a deployment supplies. A
    //     `FilesystemSessionStore` persisting bearer material would be a
    //     security regression, not a consolidation. ---
    "InMemorySessionStore",
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

    // Check each list for internal duplicates first, so a combined-length
    // mismatch below can only mean cross-list overlap (a duplicate inside one
    // list would otherwise masquerade as a disjointness failure).
    for (label, list) in [
        ("FROZEN_DEBT_INMEMORY_STORES", FROZEN_DEBT_INMEMORY_STORES),
        (
            "JUSTIFIED_KEEP_INMEMORY_STORES",
            JUSTIFIED_KEEP_INMEMORY_STORES,
        ),
    ] {
        let unique: BTreeSet<&str> = list.iter().copied().collect();
        assert_eq!(
            unique.len(),
            list.len(),
            "{label} contains duplicate entries"
        );
    }
    let frozen: BTreeSet<&str> = FROZEN_DEBT_INMEMORY_STORES
        .iter()
        .chain(JUSTIFIED_KEEP_INMEMORY_STORES)
        .copied()
        .collect();
    assert_eq!(
        frozen.len(),
        FROZEN_DEBT_INMEMORY_STORES.len() + JUSTIFIED_KEEP_INMEMORY_STORES.len(),
        "the debt and justified-keep lists must be disjoint"
    );
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
         FROZEN_DEBT_INMEMORY_STORES (or, with an audited reason, \
         JUSTIFIED_KEEP_INMEMORY_STORES) — but the intended direction is to delete these, not \
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
        "the frozen lists name stores that no longer exist: {removed:?}. A store \
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
