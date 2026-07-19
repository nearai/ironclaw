//! Anti-slippage ratchet for the deployment-mode-as-type axis (§4.4 / §10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §4.4's rule: **a deployment mode is a config value, never a type the kernel or
//! a substrate names.** Today a whole `LocalDev*` shadow runtime encodes local-dev
//! as a type family (approval/capability/lease/mount/network/outbound policy, the
//! store aliases, the root filesystem, the turn-state store). Slice B collapses
//! all of it to a `DeploymentConfig` value.
//!
//! That migration is incremental, so this test **freezes the current set of
//! pub-visible `LocalDev*` type definitions** (struct/enum/trait/type alias)
//! and fails on any change:
//!
//! - a **new** `LocalDev*` type (not in the allowlist) fails — the deployment
//!   mode must resolve to policy data at the composition edge, not grow another
//!   type;
//! - a **second definition** of a frozen name (same file or another
//!   module/crate) fails — occurrences are preserved and multiplicity is
//!   checked explicitly;
//! - **deleting** one without trimming [`FROZEN_LOCALDEV_TYPES`] also fails — so
//!   the allowlist shrinks in lock-step as Slice B lands (§10: compare set
//!   membership, never a count), and reviewers watch it get shorter.
//!
//! Scanner semantics (shared with the other §10 ratchets — see
//! [`ratchet_support`]): comments/strings stripped before matching; covers
//! `pub`/`pub(crate)`/`pub(super)`/`pub(in …)` and `unsafe`/`auto` trait
//! modifiers; skips `tests/`, `examples/`, and `benches/` trees; line-based,
//! not cfg-aware — a pub-visible definition in an inline `#[cfg(test)]` module
//! in src IS inventoried (keep test doubles under `tests/`).
//!
//! Definition of done for this axis (§4.4/§10): the allowlist reaches the empty
//! set — no `LocalDev*` type remains; local-dev is one `DeploymentConfig`
//! constant. (Scoped to `LocalDev*` specifically: the broader §4.4 `Local*` /
//! `Hosted*` name audit — Bucket 2 renames like `LocalFilesystem`→`DiskFilesystem`
//! and Bucket 3 false positives like `Locale`, `HostedMcp*`, `NodeTraceSubmission*`
//! — is a separate concern, so this ratchet stays high-signal with a clean
//! empty-set goal.)

mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, duplicate_definitions, scan_type_defs, workspace_root,
};

const KEYWORDS: &[&str] = &["struct ", "enum ", "trait ", "type "];

fn is_localdev_type(ident: &str) -> bool {
    ident.starts_with("LocalDev")
}

/// The frozen inventory of `LocalDev*` type definitions under `crates/`, as of
/// the store-consolidation ratchet (§10). Every entry is a deployment-mode-as-type
/// leak that Slice B removes by resolving mode to a `DeploymentConfig` value.
/// Remove an entry in the same PR that deletes its type; never add one.
const FROZEN_LOCALDEV_TYPES: &[&str] = &[
    // EMPTY — the §4.4 definition of done for this axis, reached by the
    // DeploymentConfig refactor: deployment mode is one config value
    // (`ironclaw_reborn_composition::deployment::DeploymentConfig`) and the
    // former `LocalDev*` shadow-runtime types are renamed to the shared
    // mechanism they always were (Builtin*/Composed*/Staged*/Synthetic*/
    // Snapshot* families). Any new `LocalDev*` type definition fails this
    // ratchet outright.
];

#[test]
fn reborn_localdev_typename_allowlist_is_frozen_and_only_shrinks() {
    let crates_dir = workspace_root().join("crates");
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crates_dir,
        KEYWORDS,
        &is_localdev_type,
        &[
            "reborn_inmemory_store_ratchet.rs",
            "reborn_localdev_typename_ratchet.rs",
        ],
        &mut found,
    );

    let frozen: BTreeSet<&str> = FROZEN_LOCALDEV_TYPES.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.keys().map(String::as_str).collect();

    let added: Vec<(&str, &Vec<TypeDefOccurrence>)> = found
        .iter()
        .filter(|(name, _)| !frozen.contains(name.as_str()))
        .map(|(name, paths)| (name.as_str(), paths))
        .collect();
    assert!(
        added.is_empty(),
        "New `LocalDev*` type definitions are banned (arch-simplification §4.4/§10): a \
         deployment mode is a `DeploymentConfig` value, never a type. Offending new types: \
         {added:?}. Resolve the mode to policy data at the composition edge instead of \
         adding a type."
    );

    let duplicated = duplicate_definitions(&found);
    assert!(
        duplicated.is_empty(),
        "Each frozen LocalDev* type name must have exactly one definition; a second \
         same-named definition elsewhere is new debt hiding behind an allowlist entry \
         (§10): {duplicated:?}"
    );

    let removed: Vec<&&str> = frozen.difference(&found_refs).collect();
    assert!(
        removed.is_empty(),
        "FROZEN_LOCALDEV_TYPES lists types that no longer exist: {removed:?}. A LocalDev* \
         type was deleted (good — Slice B progress!) — trim it from the allowlist in the \
         same PR so the ratchet keeps shrinking toward empty (§10)."
    );
}

/// Self-test for the shared scanner as this ratchet configures it: all four
/// definition keywords, restricted visibility, `unsafe`/`auto` trait modifiers,
/// and — because comments and strings are stripped before matching —
/// definition-shaped text in comments and plain/raw string literals is ignored.
#[test]
fn localdev_type_def_scanner_self_test() {
    let sample = r##"
        pub struct LocalDevWidget { x: u8 }
        pub(crate) type LocalDevAlias = u8;
        pub enum LocalDevMode { A, B }
        pub trait LocalDevPort {}
        pub unsafe trait LocalDevUnsafePort {}  // modifier tolerated
        pub(crate) unsafe trait LocalDevScopedUnsafePort {}
        struct LocalDevPrivate;              // not pub-visible -> ignored
        pub struct HostedWidget;             // not LocalDev -> ignored
        let x = "pub struct LocalDevStringLiteral";  // string literal -> ignored
        // pub struct LocalDevLineCommented  -> ignored
        /*
        pub enum LocalDevBlockCommented { A }
        */
        let raw = r#"
        pub type LocalDevRawString = u8;
        "#;
        fn build_local_dev_runtime() {}      // fn, not a type -> ignored
    "##;
    let got: Vec<String> = scan_type_defs(sample, KEYWORDS, &is_localdev_type)
        .into_iter()
        .map(|(ident, _)| ident)
        .collect();
    assert_eq!(
        got,
        vec![
            "LocalDevWidget",
            "LocalDevAlias",
            "LocalDevMode",
            "LocalDevPort",
            "LocalDevUnsafePort",
            "LocalDevScopedUnsafePort",
        ],
        "scanner must match pub-visible LocalDev* type definitions outside \
         comments and strings, in source order"
    );
}

/// Same-file multiplicity: two same-named `LocalDev*` definitions in one file
/// (e.g. a struct in one module, a type alias in another) must be flagged.
#[test]
fn localdev_same_file_duplicate_detection_self_test() {
    let sample = r#"
        mod first {
            pub struct LocalDevDupThing;
        }
        mod second {
            pub type LocalDevDupThing = u8;
        }
    "#;
    let occurrences = scan_type_defs(sample, KEYWORDS, &is_localdev_type);
    let idents: Vec<&str> = occurrences
        .iter()
        .map(|(ident, _)| ident.as_str())
        .collect();
    assert_eq!(
        idents,
        vec!["LocalDevDupThing", "LocalDevDupThing"],
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

/// Regression for the composition `factory.rs` pattern: a durable/no-durable
/// alias pair — the SAME name defined twice, each under a mutually exclusive
/// `#[cfg(...)]` — is legitimate and must NOT be flagged as a duplicate. A
/// mixed pair (only one occurrence gated) is still flagged.
#[test]
fn localdev_cfg_gated_alias_pair_is_not_a_duplicate() {
    let sample = r#"
        #[cfg(any(
            not(feature = "inmemory-turn-state"),
            any(feature = "libsql", feature = "postgres")
        ))]
        pub(crate) type LocalDevCfgPairStore = DurableImpl;
        #[cfg(not(any(feature = "libsql", feature = "postgres")))]
        pub(crate) type LocalDevCfgPairStore = VolatileImpl;
    "#;
    let occurrences = scan_type_defs(sample, KEYWORDS, &is_localdev_type);
    assert_eq!(occurrences.len(), 2, "both cfg branches must be scanned");
    assert!(
        occurrences.iter().all(|(_, cfg_gated)| *cfg_gated),
        "both branch definitions must be marked cfg-gated"
    );

    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    for (ident, cfg_gated) in occurrences {
        found.entry(ident).or_default().push(TypeDefOccurrence {
            path: PathBuf::from("crate_a/src/factory.rs"),
            cfg_gated,
        });
    }
    assert!(
        duplicate_definitions(&found).is_empty(),
        "an all-cfg-gated same-name pair is mutually exclusive, not duplicate debt"
    );

    // Mixed: one gated, one not — still duplicate debt.
    let mixed_sample = r#"
        #[cfg(feature = "libsql")]
        pub(crate) type LocalDevMixedThing = A;
        pub(crate) type LocalDevMixedThing = B;
    "#;
    let mut mixed: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    for (ident, cfg_gated) in scan_type_defs(mixed_sample, KEYWORDS, &is_localdev_type) {
        mixed.entry(ident).or_default().push(TypeDefOccurrence {
            path: PathBuf::from("crate_a/src/factory.rs"),
            cfg_gated,
        });
    }
    assert_eq!(
        duplicate_definitions(&mixed).len(),
        1,
        "a partially cfg-gated same-name pair must still be flagged"
    );
}
