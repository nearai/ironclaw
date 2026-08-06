//! Positive + negative fixtures for the **shared** source scanners in
//! `ratchet_support` (CHECKLIST WS10, the scanner-consolidation row).
//!
//! Why this binary exists. The row that ordered the consolidation ends with the
//! rule that makes it safe: *"the consolidated helpers land with their own
//! positive/negative fixtures, since every gate's non-vacuity then depends on
//! them."* Before consolidation a syntax shape each copy mishandled cost one
//! gate; now it costs every gate at once, so the shared bodies need fixtures
//! that are **not** owned by any single gate — a gate can be deleted, and its
//! fixtures with it, long after twenty other gates started depending on the
//! helper it was proving.
//!
//! `ratchet_support` itself cannot hold them: it is a `mod` compiled into ~37
//! separate integration-test binaries, so a `#[test]` there would be collected
//! and run 37 times and would report a helper regression 37 times over. One
//! binary, named `reborn_*` so `code_style.yml`'s `cargo test -p
//! ironclaw_architecture_tests reborn` name filter sees it (CHECKLIST WS10's
//! loud-inventory row, defect 6).
//!
//! **Two `#[cfg(test)]` strippers, deliberately.** The measurement that
//! motivated splitting them is in `ratchet_support`'s own module comment; what
//! this file adds is the fixture proving each family behaves the way its
//! callers depend on, *including* the shape the other family gets wrong.

#[allow(dead_code)]
mod ratchet_support;

use ratchet_support::{
    balanced_angle_close, implemented_trait_names, is_rust_identifier, strip_cfg_test_blocks,
    strip_comments_and_strings, strip_line_anchored_cfg_test_items,
};

// ---------------------------------------------------------------------------
// strip_cfg_test_blocks — the byte-scanning family
// ---------------------------------------------------------------------------

#[test]
fn byte_scanning_stripper_removes_gated_items_and_keeps_production() {
    let source = "\
pub struct Before;
#[cfg(test)]
mod tests {
    pub struct Gated;
    fn nested() { let _ = 1; }
}
pub struct After;
";
    let stripped = strip_cfg_test_blocks(source);
    assert!(
        stripped.contains("Before") && stripped.contains("After"),
        "production items on both sides of a gated block must survive: {stripped:?}"
    );
    assert!(
        !stripped.contains("Gated"),
        "a `#[cfg(test)]` item must not reach a production scan: {stripped:?}"
    );
}

/// The guard `reborn_registration_pipeline_boundary.rs`'s private copy shipped
/// without: a bodiless gated item must end at its own `;`, not brace-balance
/// from the next item's block and eat it.
#[test]
fn byte_scanning_stripper_ends_a_bodiless_gated_item_at_its_semicolon() {
    for bodiless in [
        "pub struct Before;\n#[cfg(test)]\nmod tests;\npub struct After { field: u8 }\n",
        "pub struct Before;\n#[cfg(test)]\nuse std::fmt::Debug;\npub struct After { field: u8 }\n",
    ] {
        let stripped = strip_cfg_test_blocks(bodiless);
        assert!(
            stripped.contains("After"),
            "a gated item with no block must not swallow the item after it: {stripped:?}"
        );
        assert!(
            !stripped.contains("mod tests;") && !stripped.contains("use std::fmt::Debug;"),
            "the gated declaration itself must still be removed: {stripped:?}"
        );
    }
}

/// `#[cfg(feature = "test-support")]` compiles into real builds (CI runs
/// `--all-features`), so an item behind it is a genuine edge and must survive.
#[test]
fn byte_scanning_stripper_keeps_feature_gated_items() {
    let source = "#[cfg(feature = \"test-support\")]\npub struct FeatureGated;\n";
    assert!(
        strip_cfg_test_blocks(source).contains("FeatureGated"),
        "only `#[cfg(test)]` is invisible to a shipped artifact"
    );
}

/// NEGATIVE: the documented precondition. Fed raw source, the byte scanner
/// starts a brace walk from text inside a comment or a string literal. Pinned
/// rather than fixed, because the two callers that pass raw source do so on
/// purpose (`ratchet_support` names them) and closing it would change what they
/// count. If this test ever goes green the hazard changed shape — re-derive the
/// callers' safety argument, do not delete the test.
#[test]
fn byte_scanning_stripper_is_unsound_on_raw_source_which_is_why_callers_pre_strip() {
    let source = "\
#[cfg(test)]
mod tests {
    /// an unbalanced brace in a doc comment: {
    const S: &str = \"{{{\";
}
pub struct Production;
";
    assert!(
        !strip_cfg_test_blocks(source).contains("Production"),
        "the raw-source hazard is the reason every sound caller composes this with \
         strip_comments_and_strings first — if raw input now works, the composition rule in \
         ratchet_support needs rewriting, not deleting"
    );
    assert!(
        strip_cfg_test_blocks(&strip_comments_and_strings(source)).contains("Production"),
        "the documented composition must survive braces hidden in comments and strings"
    );
}

// ---------------------------------------------------------------------------
// strip_line_anchored_cfg_test_items — the raw-source family
// ---------------------------------------------------------------------------

#[test]
fn line_anchored_stripper_removes_gated_items_and_keeps_production() {
    let source = "\
pub struct Before;
#[cfg(test)]
mod tests {
    pub struct Gated;
}
pub struct After;
";
    let stripped = strip_line_anchored_cfg_test_items(source);
    assert!(stripped.contains("Before") && stripped.contains("After"));
    assert!(!stripped.contains("Gated"));
}

/// The property its two callers depend on and the byte-scanning family cannot
/// provide: fed **raw** source, a `#[cfg(test)]` written inside a comment or a
/// string literal must not strip anything. `reborn_extension_specificity`
/// treats a vendor name in a comment as a violation and so cannot pre-strip;
/// if this regressed, a vendor name could be hidden from that gate by writing
/// `#[cfg(test)]` in a comment above it.
#[test]
fn line_anchored_stripper_ignores_a_marker_inside_a_comment_or_string() {
    let source = "\
// #[cfg(test)]
pub struct AfterCommentMarker;
const S: &str = \"#[cfg(test)]\";
pub struct AfterStringMarker;
";
    let stripped = strip_line_anchored_cfg_test_items(source);
    assert!(
        stripped.contains("AfterCommentMarker") && stripped.contains("AfterStringMarker"),
        "a marker that is not the first token of its line is not an attribute: {stripped:?}"
    );
}

/// NEGATIVE, and the reason the families are not merged: the byte scanner gets
/// the same raw input wrong, in the fail-open direction (it deletes production
/// code the specificity gate is supposed to scan).
#[test]
fn the_two_strippers_disagree_on_raw_source_and_that_is_the_split() {
    let source = "\
// #[cfg(test)]
mod not_a_gated_module {
    pub struct VendorNameLivesHere;
}
";
    assert!(
        strip_line_anchored_cfg_test_items(source).contains("VendorNameLivesHere"),
        "the raw-source family must keep it"
    );
    assert!(
        !strip_cfg_test_blocks(source).contains("VendorNameLivesHere"),
        "the byte-scanning family must be shown to lose it — if this fires, the two families \
         have converged and the split in ratchet_support can be revisited deliberately"
    );
}

#[test]
fn line_anchored_stripper_ends_a_bodiless_gated_item_at_its_semicolon() {
    let stripped = strip_line_anchored_cfg_test_items(
        "pub struct A;\n#[cfg(test)]\nmod tests;\npub struct B;\n",
    );
    assert!(stripped.contains("A") && stripped.contains("B"));
    assert!(!stripped.contains("mod tests;"));
}

// ---------------------------------------------------------------------------
// balanced_angle_close
// ---------------------------------------------------------------------------

#[test]
fn balanced_angle_close_balances_nested_and_rejects_truncated() {
    assert_eq!(balanced_angle_close("<T>"), Some(2));
    // Nested: taking the FIRST `>` would leave `> Port`, not an identifier, and
    // the impl would be skipped — a gate enforcing nothing for it.
    let nested = "<T: Iterator<Item = X>>";
    assert_eq!(balanced_angle_close(nested), Some(nested.len() - 1));
    // A `->` inside a bound is a return arrow, not a close.
    let arrow = "<F: Fn(&str) -> bool>";
    assert_eq!(balanced_angle_close(arrow), Some(arrow.len() - 1));
    // NEGATIVE: never balances.
    assert_eq!(balanced_angle_close("<T: Iterator<Item = X>"), None);
}

// ---------------------------------------------------------------------------
// implemented_trait_names
// ---------------------------------------------------------------------------

#[test]
fn implemented_trait_names_reads_every_impl_shape_and_rejects_non_impls() {
    let source = "\
impl Plain for Thing {}
impl ironclaw_assistant::Qualified<T> for Thing {}
impl<'a, T: Iterator<Item = X>> Generic for Host<T> {}
impl<F: Fn(&str) -> bool> WithArrowBound for Host<F> {}
impl Inherent { fn method() {} }
#[cfg(test)]
mod tests {
    impl GatedOnly for Double {}
}
";
    let found = implemented_trait_names(source);
    for expected in ["Plain", "Qualified", "Generic", "WithArrowBound"] {
        assert!(found.contains(expected), "missed {expected}: {found:?}");
    }
    assert!(
        !found.contains("Inherent"),
        "an inherent impl has no `for` and is not an implemented trait: {found:?}"
    );
    assert!(
        !found.contains("GatedOnly"),
        "a `#[cfg(test)]` impl is not a production edge: {found:?}"
    );
}

// ---------------------------------------------------------------------------
// is_rust_identifier
// ---------------------------------------------------------------------------

#[test]
fn is_rust_identifier_accepts_identifiers_and_rejects_fragments() {
    for accepted in ["Foo", "_private", "Foo2", "snake_case"] {
        assert!(is_rust_identifier(accepted), "{accepted} is an identifier");
    }
    // The fragments a substring-oriented scan produces when it reads a shape it
    // does not understand. Accepting any of these would let a gate record a
    // non-name as a trait and satisfy a pin with it.
    for rejected in ["", "2Foo", "a::b", "Foo<T>", "Foo ", "&Foo", "-"] {
        assert!(
            !is_rust_identifier(rejected),
            "{rejected:?} must not read as an identifier"
        );
    }
}
