//! Anti-slippage ratchet for the capability-path DTO collapse (§3/§9/§10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
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

use std::collections::{BTreeMap, BTreeSet};

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, duplicate_definitions, scan_type_defs, workspace_root,
};

const KEYWORDS: &[&str] = &["struct ", "enum ", "trait ", "type "];

/// The capability-path mirror DTOs the collapse retires (§3.1). Each is a
/// request/result shape that `Invocation`/`Authorized`/`Resolution` subsume.
/// Remove an entry in the same PR that deletes its type; never add one.
const FROZEN_COLLAPSE_DTOS: &[&str] = &[
    // ── request side: the ~5 near-identical shapes (§1.1) ──
    // turns (§1.1 hop 1 — the loop's expression; `LoopRequest` replaces it, §3.1)
    "CapabilityInvocation",
    // host_runtime (the upper mirrors)
    "RuntimeCapabilityRequest",
    "RuntimeCapabilityResumeRequest",
    "RuntimeCapabilityAuthResumeRequest",
    // capabilities (the kernel-facing mirrors — `Invocation` replaces them)
    "CapabilityInvocationRequest",
    "CapabilityResumeRequest",
    "CapabilityAuthResumeRequest",
    // dispatcher (`Authorized` + resolved handles replace it, §3.1)
    "RuntimeAdapterRequest",
    // ── result side: the overloaded ten-variant enum (§1.2 → `Resolution`) ──
    // `CapabilityOutcome` (and its `CapabilityBatchOutcome`/`CapabilityResultMessage`/
    // `CapabilityFailure`/`CapabilityDenied`/`ProcessHandleSummary` payloads) are
    // DELETED (§5.3 Stage 2b): producers emit `host_api::Resolution` directly via
    // the `ironclaw_turns::run_profile::resolution::*` constructors. The result-lane
    // collapse is complete; only the request-side shapes above remain to retire.
];

/// Matches exactly the frozen collapse-target names (exact identifier, not a
/// prefix — `CapabilityOutcomeKind` or `RuntimeAdapterRequestBuilder` would not
/// match). The set is the allowlist itself: this axis has no shared name pattern,
/// so the ratchet's job is to force the known shapes to *shrink to empty*, and to
/// flag a re-declaration (a second definition of a frozen name).
fn is_collapse_dto(ident: &str) -> bool {
    FROZEN_COLLAPSE_DTOS.contains(&ident)
}

#[test]
fn reborn_capability_dto_allowlist_is_frozen_and_only_shrinks() {
    let crates_dir = workspace_root().join("crates");
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crates_dir,
        KEYWORDS,
        &is_collapse_dto,
        &["reborn_capability_dto_collapse_ratchet.rs"],
        &mut found,
    );

    let frozen: BTreeSet<&str> = FROZEN_COLLAPSE_DTOS.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.keys().map(String::as_str).collect();

    // A frozen name appearing a SECOND time = a re-declared mirror (§1.1
    // Mechanism 1). This is the "no new mirror hiding behind an allowlist entry"
    // guard the exact-name predicate can enforce.
    let duplicated = duplicate_definitions(&found);
    assert!(
        duplicated.is_empty(),
        "A collapse-target DTO is defined more than once — a re-declared mirror \
         (arch-simplification §1.1 Mechanism 1 / §10). Import the one definition \
         instead of re-declaring it: {duplicated:?}"
    );

    let removed: Vec<&&str> = frozen.difference(&found_refs).collect();
    assert!(
        removed.is_empty(),
        "FROZEN_COLLAPSE_DTOS lists types that no longer exist: {removed:?}. The \
         capability-path collapse deleted one (good — §3 progress!) — trim it from \
         the allowlist in the same PR so this ratchet shrinks toward empty (§10). \
         When the last entry goes, delete this file."
    );

    // Unlike the pattern-matching ratchets (which keep guarding against NEW
    // matches at an empty allowlist), an exact-name ratchet with an empty list
    // can never fail again — so the end state is enforced as deletion, not an
    // eternally-green dead test.
    assert!(
        !FROZEN_COLLAPSE_DTOS.is_empty(),
        "The capability-path DTO collapse is complete (§3 end state reached). \
         This exact-name ratchet can no longer catch anything — delete this file."
    );
}

/// Self-test for the predicate as configured: exact-name only.
#[test]
fn collapse_dto_predicate_is_exact_name() {
    let sample = r#"
        pub struct RuntimeCapabilityRequest { a: u8 }     // frozen -> flagged
        pub struct CapabilityAuthResumeRequest { a: u8 }  // frozen -> flagged
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
