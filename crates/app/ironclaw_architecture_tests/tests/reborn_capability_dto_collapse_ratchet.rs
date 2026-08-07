//! Anti-slippage ratchet for the capability-path DTO collapse (§3/§9/§10 of
//! `docs/reborn/contracts/capability-access.md`).
//!
//! §1.1 showed a single capability call re-wrapped through ~5 near-identical
//! request shapes plus an overloaded ten-variant result enum. §3 collapses those
//! onto the `host_api` vocabulary (`Invocation`/`Authorized`/`Resolution`). §9 is
//! explicit that during the migration the type count *rises before it falls*
//! (~14 → ~18 → ~11) while the new vocabulary and the old shapes coexist, and
//! that **"the §10 mirror-DTO ratchet's allowlist is what makes this safe: the
//! old [shapes] are frozen entries that may only disappear."**
//!
//! The collapse COMPLETED (#6447 retired the last request DTOs), so this file
//! is no longer the shrinking freeze that §9 describes — it is the permanent
//! zero-gate that survives it: [`RETIRED_COLLAPSE_DTOS`] names the retired
//! mirror shapes, and the test fails if any of them is ever re-declared
//! (the exact §1.1 Mechanism 1 failure, attempted after the fact).
//!
//! Two of the ten originally-frozen names are deliberately NOT here:
//! `CapabilityOutcome` (deleted by #6299 before the retired-list conversion)
//! and `CapabilityDispatchRequest` (its removal was a blessing, not a
//! deletion — it survives as the canonical port type in
//! `ironclaw_host_api::dispatch`). Reintroducing the former fires nothing;
//! add it below if that ever becomes a live hazard.
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
fn reborn_capability_dto_names_stay_retired() {
    let crates_dir = workspace_root().join("crates");
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crates_dir,
        KEYWORDS,
        &is_collapse_dto,
        &["reborn_capability_dto_collapse_ratchet.rs"],
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
