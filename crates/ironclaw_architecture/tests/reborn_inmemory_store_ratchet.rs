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
//! Definition of done for this axis (§10): the **debt** shrinks to empty — every
//! persistence-store duplicate becomes `Filesystem*Store<InMemoryBackend>` in
//! tests. The mechanical consolidations (A1–A8: approvals, authorization,
//! processes, run-state, budget-gate, and the whole outbound family) are done;
//! see the annotated `FROZEN_INMEMORY_STORES` below for the per-entry status of
//! the remainder. Note two entries (`InMemoryBoundedSubagentGoalStore`,
//! `InMemoryOpenAiCompatRefStore`) are **justified bounded caches**, not
//! persistence debt — a future PR may formally split them into a justified-keep
//! list; for now they stay frozen with a do-not-consolidate note. Until then
//! this frozen set is the contract.

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
/// `crates/`. Remove an entry in the same PR that deletes its store; never add
/// one. Comments are stripped by the scanner, so the per-entry status notes
/// below are documentation only — the enforced contract is the string set.
///
/// **Status of the remainder (assessed 2026-07-18, after the mechanical §4.3
/// slices A1–A8 landed the approvals/authorization/processes/run-state/budget-gate
/// and the whole outbound family).** The clean mechanical consolidations are
/// DONE; every entry still here is blocked on non-mechanical work OR is a
/// justified keep — except the trailing pub(crate) trio, which is not yet
/// individually triaged. Triaged entries are annotated with WHAT they need, so
/// the next contributor picks up a scoped task instead of re-deriving the
/// blocker; untriaged entries say so explicitly.
const FROZEN_INMEMORY_STORES: &[&str] = &[
    // --- turns cluster: DEFERRED, not mechanical. `InMemoryTurnStateStore` is the
    //     `inmemory-turn-state` production runtime authority (pessimistic Mutex,
    //     no-CAS-livelock); a `FilesystemTurnStateStore<InMemoryBackend>` swap needs
    //     a concurrency stress test PROVING it keeps the no-livelock property first.
    //     `FilesystemCheckpointStateStore` already EXISTS in `ironclaw_loop_host`
    //     (contract-tested, composition-wired) — that entry only needs the test-seam
    //     swap + allowlist trim; LoopCheckpoint/InstructionMaterialization still need
    //     a filesystem variant BUILT (cross-crate in `ironclaw_loop_host`). ---
    "InMemoryTurnStateStore",
    "InMemoryCheckpointStateStore",
    "InMemoryLoopCheckpointStore",
    "InMemoryInstructionMaterializationStore",
    // --- JUSTIFIED KEEPS — bounded in-memory caches serving the test/no-durable
    //     fallback role. Durable production variants ALREADY EXIST and are wired
    //     (`FilesystemSubagentGoalStore` in the libSQL/Postgres runner adapters;
    //     `FilesystemOpenAiCompatRefStore` in OpenAI-compatible serving) — these
    //     in-memory types are not missing consolidations, they are the bounded
    //     volatile role next to those stores. Do NOT swap them for a durable
    //     store in tests that specifically exercise the bounded/evicting cache
    //     semantics. ---
    //   BoundedSubagentGoal: capacity-bounded, evict-oldest (VecDeque insertion
    //     order) cache of in-flight subagent-spawn goals — goal_store.rs.
    "InMemoryBoundedSubagentGoalStore",
    //   OpenAiCompatRef: capacity-bounded with oldest-created eviction (evicts the
    //     minimum `created_at`; reads do not refresh recency — NOT an LRU) AND the
    //     crate's documented filesystem-free default so contract-only consumers pull
    //     no `ironclaw_filesystem` dep (openai_compat CLAUDE.md).
    "InMemoryOpenAiCompatRefStore",
    // --- BLOCKED — cross-crate placement. `FilesystemExtensionInstallationStore`
    //     exists but in high `ironclaw_reborn_composition` and depends on a
    //     composition-internal contract registry, so it can't move DOWN to
    //     `ironclaw_extensions` (whose own tests need an in-memory store). Needs the
    //     filesystem store (or its contract dep) relocated first. Prod already wires
    //     Filesystem. ---
    "InMemoryExtensionInstallationStore",
    // --- SECURITY-SENSITIVE — secrets subsystem; deliberate careful work, not a
    //     mechanical swap. ---
    "InMemorySecretStore",
    // --- BUILD-FIRST — no filesystem variant exists. `InMemorySessionStore`
    //     (webui login sessions, TTL-expiring bearer tokens) would gain restart
    //     durability from a `FilesystemSessionStore`, but that store must be BUILT
    //     (auth-adjacent — handle with care). ~63 usages. ---
    "InMemorySessionStore",
    // --- pub(crate) stores the visibility-aware scanner also inventories
    //     (crate-private; not yet individually triaged — assess build-vs-justified
    //     when picked up, same as the peripheral set above). ---
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
