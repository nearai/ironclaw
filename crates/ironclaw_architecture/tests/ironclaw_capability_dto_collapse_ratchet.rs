//! Anti-slippage ratchet for the capability-path DTO collapse (§3/§9/§10 of
//! `docs/ironclaw/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §1.1 showed a single capability call re-wrapped through ~5 near-identical
//! request shapes plus an overloaded ten-variant result enum. §3 collapses those
//! onto the `host_api` vocabulary (`Invocation`/`Authorized`/`Resolution`). §9 is
//! explicit that during the migration the type count *rises before it falls*
//! (~14 → ~18 → ~11) while the new vocabulary and the old shapes coexist, and
//! that **"the §10 mirror-DTO ratchet's allowlist is what makes this safe: the
//! old [shapes] are frozen entries that may only disappear."**
//!
//! This is that ratchet. It freezes the current set of collapse-target DTOs and
//! fails on any change:
//!
//! - a **second definition** of a frozen name (a downstream crate re-declaring an
//!   upstream request "for decoupling" — the exact §1.1 Mechanism 1 failure) —
//!   flagged by the multiplicity check;
//! - **deleting** one without trimming [`FROZEN_COLLAPSE_DTOS`] also fails — so
//!   the allowlist shrinks in lock-step as the collapse lands (§10: compare set
//!   membership, never a count), and reviewers watch it get shorter toward the
//!   §3 end state (`Invocation` + the surviving per-lane requests + the five
//!   result channels — zero mirrors).
//!
//! Definition of done for this axis: every entry below is deleted (its fields
//! now carried by `Invocation`/`Authorized`, its result variants by
//! `Resolution`), and this file is deleted with the last of them (enforced —
//! the test fails on an empty allowlist).
//!
//! Owner: the §3 capability-path collapse slice series under the #6168
//! umbrella (authorize → dispatch → result-channel migration) trims this list
//! in the same PRs that delete the types; reviewers hold additions to zero.
//!
//! Scanner semantics (shared with the other §10 ratchets — see
//! [`ratchet_support`]): comments/strings stripped before matching; covers
//! `pub`/`pub(crate)`/`pub(super)`/`pub(in …)`; skips `tests/`/`examples/`/
//! `benches/`; line-based, not cfg-aware.

mod ratchet_support;

use std::collections::BTreeMap;

use ratchet_support::{TypeDefOccurrence, collect_type_defs, scan_type_defs, workspace_root};

const KEYWORDS: &[&str] = &["struct ", "enum ", "trait ", "type "];

/// Retired capability-path mirror DTO names (§3.1). These request/result shapes
/// are subsumed by `Invocation`/`Authorized`/`Resolution` or by tuple parts at
/// the object-safe runtime boundary. They must not reappear.
const RETIRED_COLLAPSE_DTOS: &[&str] = &[
    "CapabilityInvocation",
    "RuntimeCapabilityRequest",
    "RuntimeCapabilityResumeRequest",
    "RuntimeCapabilityAuthResumeRequest",
    "CapabilityInvocationRequest",
    "CapabilityResumeRequest",
    "CapabilityAuthResumeRequest",
    "RuntimeAdapterRequest",
];

/// Matches exactly the frozen collapse-target names (exact identifier, not a
/// prefix — `CapabilityOutcomeKind` or `RuntimeAdapterRequestBuilder` would not
/// match). The set is the allowlist itself: this axis has no shared name pattern,
/// so the ratchet's job is to force the known shapes to *shrink to empty*, and to
/// flag a re-declaration (a second definition of a frozen name).
fn is_collapse_dto(ident: &str) -> bool {
    RETIRED_COLLAPSE_DTOS.contains(&ident)
}

#[test]
fn ironclaw_capability_dto_names_stay_retired() {
    let crates_dir = workspace_root().join("crates");
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crates_dir,
        KEYWORDS,
        &is_collapse_dto,
        &["ironclaw_capability_dto_collapse_ratchet.rs"],
        &mut found,
    );

    assert!(
        found.is_empty(),
        "Retired capability-path mirror DTO names were reintroduced. Use \
         `Invocation`/`Authorized`/`Resolution`, `LoopRequest`, runtime tuple \
         parts, or the private lane request instead: {found:?}"
    );
}

/// Self-test for the predicate as configured: exact-name only.
#[test]
fn collapse_dto_predicate_is_exact_name() {
    let sample = r#"
        pub struct RuntimeCapabilityRequest { a: u8 }     // retired -> flagged
        pub struct CapabilityAuthResumeRequest { a: u8 }  // retired -> flagged
        pub struct RuntimeAdapterRequestBuilder;          // suffix -> NOT flagged
        pub struct Invocation;                            // the target -> NOT flagged
        pub struct CapabilityDispatchResult;              // sibling result -> NOT flagged
    "#;
    let got: Vec<String> = scan_type_defs(sample, KEYWORDS, &is_collapse_dto)
        .into_iter()
        .map(|(ident, _)| ident)
        .collect();
    assert_eq!(
        got,
        vec!["RuntimeCapabilityRequest", "CapabilityAuthResumeRequest"]
    );
}
