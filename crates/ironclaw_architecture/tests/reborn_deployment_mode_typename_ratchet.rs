//! Anti-slippage ratchet for the deployment-mode-name axis, the broader
//! companion to [`reborn_localdev_typename_ratchet`] (§4.4 / §10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §4.4 mandates one enforcement test: **"no public type name contains
//! `Local`/`LocalDev`/`Hosted`/`Enterprise`"** — a deployment mode is a
//! `DeploymentConfig` value, never a type the kernel or a substrate names. The
//! sibling `reborn_localdev_typename_ratchet` owns the `LocalDev*` shadow-runtime
//! family (shrinking to empty as Slice B lands). This ratchet owns the OTHER
//! three prefixes the doc names, which that ratchet deliberately scoped out as "a
//! separate concern":
//!
//! - `Enterprise*` — **none exist** (achieved). The empty allowlist locks that
//!   in: a new `Enterprise*` type (an `EnterpriseTierPolicy` deployment-mode
//!   leak) fails the "no new" check.
//! - `Hosted*` — the current set is all `HostedMcp*` (+ discovery/egress), a
//!   Bucket-3 **false positive**: "hosted MCP" is a real domain concept (a
//!   platform-hosted MCP server, vs a self-hosted one), NOT a `HostedDev`/hosted-
//!   TIER deployment-mode leak. Justified-keep; frozen so a genuine
//!   `HostedTierRuntime`-style leak can't slip in behind them.
//! - `Local*` (excluding `LocalDev*`, owned by the sibling ratchet, and `Locale*`,
//!   a Bucket-3 localization false positive) — the `LocalTriggerAccess*` family
//!   is genuine Bucket-1 debt: §4.4 folds the `local_trigger_access` module into
//!   "seed the owner grant from config at boot," a policy value. Shrinks as that
//!   lands. `LocalInvocationServicesResolver` awaits a rename (its correct name
//!   is a design call — it wires host OR sandbox ports).
//!
//! Scanner semantics (shared with the other §10 ratchets — see
//! [`ratchet_support`]): comments/strings stripped before matching; covers
//! `pub`/`pub(crate)`/`pub(super)`/`pub(in …)` and `unsafe`/`auto` trait
//! modifiers; skips `tests/`, `examples/`, and `benches/` trees; line-based, not
//! cfg-aware. Definition of done: the `Local*` debt shrinks to empty; the
//! `Hosted*` false positives stay (trim only if a type is genuinely deleted);
//! `Enterprise*` stays empty.

mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, duplicate_definitions, scan_type_defs, workspace_root,
};

const KEYWORDS: &[&str] = &["struct ", "enum ", "trait ", "type "];

/// Matches deployment-mode-name candidates for the three prefixes this ratchet
/// owns: `Hosted*`, `Enterprise*`, and `Local*` EXCEPT `LocalDev*` (the sibling
/// ratchet's domain) and `Locale*` (a localization false positive). `starts_with`
/// (not `contains`) so mid-word matches like `HookLocalId` are not flagged.
fn is_other_mode_prefixed(ident: &str) -> bool {
    if ident.starts_with("Hosted") || ident.starts_with("Enterprise") {
        return true;
    }
    ident.starts_with("Local") && !ident.starts_with("LocalDev") && !ident.starts_with("Locale")
}

/// The frozen inventory of pub-visible `Hosted*`/`Enterprise*`/`Local*`
/// (non-`LocalDev`, non-`Locale`) type definitions under `crates/`. Comments are
/// stripped by the scanner, so the per-entry status notes are documentation only;
/// the enforced contract is the string set. Trim an entry in the same PR that
/// deletes/renames its type.
const FROZEN_OTHER_MODE_TYPES: &[&str] = &[
    // --- Hosted*: JUSTIFIED (Bucket-3 false positive) — "hosted MCP" is a real
    //     domain concept (platform-hosted MCP server), not a deployment-mode tier.
    "HostedMcpDiscoveredTool",
    "HostedMcpDiscoveredToolAnnotations",
    "HostedMcpDiscoveryEgress",
    "HostedMcpDiscoveryError",
    "HostedMcpEndpoint",
    // --- Local* (non-LocalDev): Bucket-1 DEBT — the `local_trigger_access` module
    //     folds to "seed owner grant from config at boot" (§4.4). Shrinks as that
    //     lands.
    "LocalTriggerAccessBootstrap",
    "LocalTriggerAccessBootstrapConfig",
    "LocalTriggerAccessReconciliation",
    "LocalTriggerAccessRole",
    "LocalTriggerAccessSeed",
    "LocalTriggerAccessSource",
    "LocalTriggerAccessStatus",
    "LocalTriggerAccessStore",
    // --- Local*: pending rename — its correct name is a design call (wires host
    //     OR sandbox process ports, so "Local…" understates it).
    "LocalInvocationServicesResolver",
];

#[test]
fn reborn_other_mode_typename_allowlist_is_frozen() {
    let crates_dir = workspace_root().join("crates");
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crates_dir,
        KEYWORDS,
        &is_other_mode_prefixed,
        &[
            "reborn_inmemory_store_ratchet.rs",
            "reborn_localdev_typename_ratchet.rs",
            "reborn_deployment_mode_typename_ratchet.rs",
        ],
        &mut found,
    );

    let frozen: BTreeSet<&str> = FROZEN_OTHER_MODE_TYPES.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.keys().map(String::as_str).collect();

    let added: Vec<(&str, &Vec<TypeDefOccurrence>)> = found
        .iter()
        .filter(|(name, _)| !frozen.contains(name.as_str()))
        .map(|(name, paths)| (name.as_str(), paths))
        .collect();
    assert!(
        added.is_empty(),
        "New `Hosted*`/`Enterprise*`/`Local*` (non-LocalDev) type definitions are banned \
         (arch-simplification §4.4/§10): a deployment mode is a `DeploymentConfig` value, \
         never a type. Offending new types: {added:?}. If this is a genuine domain type \
         that only LOOKS like a mode leak (e.g. another `HostedMcp*`), justify it in review \
         and add it to FROZEN_OTHER_MODE_TYPES; otherwise resolve the mode to policy data."
    );

    let duplicated = duplicate_definitions(&found);
    assert!(
        duplicated.is_empty(),
        "Each frozen name must have exactly one definition; a second same-named definition \
         elsewhere is new debt hiding behind an allowlist entry (§10): {duplicated:?}"
    );

    let removed: Vec<&&str> = frozen.difference(&found_refs).collect();
    assert!(
        removed.is_empty(),
        "FROZEN_OTHER_MODE_TYPES lists types that no longer exist: {removed:?}. A type was \
         deleted or renamed (good) — trim it from the allowlist in the same PR."
    );
}

/// Self-test for the predicate as this ratchet configures it: it flags
/// `Hosted*`/`Enterprise*`/`Local*` but excludes `LocalDev*` (sibling ratchet),
/// `Locale*` (localization false positive), and mid-word `Local` (e.g.
/// `HookLocalId`).
#[test]
fn other_mode_predicate_self_test() {
    let sample = r##"
        pub struct HostedMcpEndpoint;            // Hosted* -> flagged
        pub struct EnterpriseTierPolicy;         // Enterprise* -> flagged
        pub struct LocalTriggerAccessSeed;       // Local* (non-Dev) -> flagged
        pub struct LocalDevApprovalGatePolicy;   // LocalDev* -> sibling ratchet, NOT flagged
        pub struct LocaleError;                  // Locale* -> localization, NOT flagged
        pub struct HookLocalId;                  // mid-word Local -> NOT flagged
        pub struct DiskFilesystem;               // no mode prefix -> NOT flagged
    "##;
    let got: Vec<String> = scan_type_defs(sample, KEYWORDS, &is_other_mode_prefixed)
        .into_iter()
        .map(|(ident, _)| ident)
        .collect();
    assert_eq!(
        got,
        vec![
            "HostedMcpEndpoint",
            "EnterpriseTierPolicy",
            "LocalTriggerAccessSeed",
        ]
    );
}
