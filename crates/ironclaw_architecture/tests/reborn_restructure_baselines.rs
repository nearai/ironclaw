//! WS0 baseline record for the target-architecture restructure (epic #3773,
//! workstream #6920 — `docs/reborn/target-architecture/CHECKLIST.md` WS0).
//!
//! CHECKLIST WS0 names five ratchets that must not regress while the
//! restructure runs. Three of them are recorded beside the test that owns the
//! list being measured, because there the constant and the data are in one
//! file and cannot drift apart:
//!
//! | baseline | recorded in |
//! |---|---|
//! | `LAYER_MATRIX_EXCEPTIONS` count (WS0 20) | `reborn_dependency_boundaries.rs` |
//! | extension-specificity allowlist size (WS0 130) | `reborn_extension_specificity.rs` |
//! | production-struct dead-code inventory (WS0 82 paths / 283 members) | `reborn_struct_test_support_ratchet.rs` |
//!
//! Only the WS0 records are quoted above. The live values are the owning
//! files' constants and ratchet down as waves land; a "now N" quoted in this
//! header rotted unnoticed (it read "now 15" while the live register was 6),
//! so this file no longer carries one — one definition per metric, owned by
//! the gate that enforces it, applies to the prose too.
//!
//! The remaining two — composition mass and the integration-coverage floor —
//! are enforced by shell gates over committed manifests, wired into CI
//! (`code_style.yml` → `scripts/ci/check-composition-budget.sh`,
//! `reborn-tests.yml` → `scripts/ci/reborn-coverage-ratchet.sh`). This file
//! records their WS0 values and pins that those gates stay **armed**: a
//! ratchet quietly flipped to dry-run, or a manifest moved out from under a
//! path-keyed gate, is exactly how the restructure would regress unobserved
//! (the failure mode CHECKLIST WS10 flags for every path-keyed gate).
//!
//! It deliberately does **not** re-implement either metric. One definition per
//! metric, owned by the gate that enforces it; a second implementation here
//! would drift and then lie.
//!
//! Every number below was measured from this checkout with the command
//! recorded beside it — never copied from the design documents. To print the
//! WS0-vs-today report:
//!
//! ```text
//! cargo test -p ironclaw_architecture --test reborn_restructure_baselines -- --nocapture
//! ```

// Only `workspace_root` is reachable from this binary; the scanners in the
// shared module belong to the sibling ratchet binaries.
#[allow(dead_code)]
mod ratchet_support;

use ratchet_support::workspace_root;

/// The tree every number in this file was measured from.
const WS0_MEASURED_FROM: &str = "origin/main @ ae0989c37 (2026-07-30)";

/// Composition mass, from `bash scripts/ci/check-composition-budget.sh --print`:
/// "composition share: 6.58% (658 bp) — 43936 / 667978 LOC".
///
/// The metric is production `.rs` LOC of `crates/ironclaw_reborn_composition/src`
/// over the same measure of every `crates/*/src` tree (test-only files excluded
/// from both sides — see `scripts/ci/composition-budget.toml` for the full
/// definition). CHECKLIST WS6 re-baselines the gate's ceiling once the eviction
/// inventory lands; this record is what "before" was.
const WS0_COMPOSITION_SRC_LOC: usize = 43_936;
const WS0_COMPOSITION_DENOMINATOR_LOC: usize = 667_978;
const WS0_COMPOSITION_SHARE_BP: usize = 658;

/// Composition dispatch, from the same `--print` run: "composition dispatch:
/// 827 Arc<dyn> (governed prod, excl slack/extension_host)".
const WS0_COMPOSITION_ARC_DYN_SITES: usize = 827;

/// Integration-coverage floor, read from `tests/integration/coverage-floor.toml`
/// `[global].floor_percent` at the WS0 commit (captured there from PR #6886's
/// merged CI artifact: aggregate 315436 / 368757 lines).
///
/// Unlike the other four this one is a *floor* rather than a ceiling: it moves
/// in whichever direction the same-PR recapture workflow in that file's header
/// dictates, so this record is a reference point for the report below, not a
/// bound. The bound is the gate.
const WS0_INTEGRATION_COVERAGE_FLOOR_PERCENT: f64 = 85.54;

/// The two committed manifests this file pins, relative to the workspace root.
const COMPOSITION_BUDGET_MANIFEST: &str = "scripts/ci/composition-budget.toml";
const COVERAGE_FLOOR_MANIFEST: &str = "tests/integration/coverage-floor.toml";

fn parse_manifest(relative: &str) -> toml::Table {
    let path = workspace_root().join(relative);
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{relative} must be readable — the WS0 baselines and the CI gate that enforces \
             them both key off this exact path; if it moved, repoint the gate, its workflow, \
             and this record together (CHECKLIST WS10 path-keyed gates): {error}"
        )
    });
    contents
        .parse::<toml::Table>()
        .unwrap_or_else(|error| panic!("{relative} must be valid TOML: {error}"))
}

fn table<'a>(manifest: &'a toml::Table, relative: &str, key: &str) -> &'a toml::Table {
    manifest
        .get(key)
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("{relative} must declare a [{key}] table"))
}

fn integer(table: &toml::Table, relative: &str, key: &str) -> i64 {
    table
        .get(key)
        .and_then(toml::Value::as_integer)
        .unwrap_or_else(|| panic!("{relative} [{key}] must be an integer"))
}

fn enforcing(table: &toml::Table, relative: &str) -> bool {
    table
        .get("enforce")
        .and_then(toml::Value::as_bool)
        .unwrap_or_else(|| panic!("{relative} must declare a boolean `enforce` flag"))
}

/// The two externally-enforced WS0 ratchets are armed, their manifests are
/// where the gates expect them, and the recorded WS0 measurements are
/// consistent with the budgets that govern them.
#[test]
fn reborn_restructure_baseline_ratchets_stay_armed() {
    let budget = parse_manifest(COMPOSITION_BUDGET_MANIFEST);
    let gate = table(&budget, COMPOSITION_BUDGET_MANIFEST, "gate");
    let ceiling_bp = integer(gate, COMPOSITION_BUDGET_MANIFEST, "ceiling_bp");
    let tolerance_bp = integer(gate, COMPOSITION_BUDGET_MANIFEST, "tolerance_bp");
    let arc_dyn_ceiling = integer(gate, COMPOSITION_BUDGET_MANIFEST, "arc_dyn_ceiling");
    let arc_dyn_tolerance = integer(gate, COMPOSITION_BUDGET_MANIFEST, "arc_dyn_tolerance");

    assert!(
        enforcing(gate, COMPOSITION_BUDGET_MANIFEST),
        "the composition mass/dispatch ratchet is no longer enforcing. It is one of the five \
         WS0 baselines the restructure is measured against (CHECKLIST WS0); a dry-run gate \
         cannot keep composition from re-accreting behavior while it is being emptied. Restore \
         `[gate].enforce = true` in {COMPOSITION_BUDGET_MANIFEST}."
    );

    let effective_ceiling_bp = ceiling_bp + tolerance_bp;
    assert!(
        i64::try_from(WS0_COMPOSITION_SHARE_BP).is_ok_and(|share| share <= effective_ceiling_bp),
        "the WS0 composition-mass record ({WS0_COMPOSITION_SHARE_BP} bp) now exceeds the gate's \
         effective ceiling ({effective_ceiling_bp} bp). Either the ceiling was tightened past \
         the recorded baseline — re-measure with `bash \
         scripts/ci/check-composition-budget.sh --print` and update the WS0 record — or the \
         ceiling is wrong."
    );
    assert!(
        i64::try_from(WS0_COMPOSITION_ARC_DYN_SITES)
            .is_ok_and(|sites| sites <= arc_dyn_ceiling + arc_dyn_tolerance),
        "the WS0 composition-dispatch record ({WS0_COMPOSITION_ARC_DYN_SITES} Arc<dyn>) now \
         exceeds the gate's effective ceiling ({}). Re-measure and update the WS0 record.",
        arc_dyn_ceiling + arc_dyn_tolerance
    );

    let coverage = parse_manifest(COVERAGE_FLOOR_MANIFEST);
    let global = table(&coverage, COVERAGE_FLOOR_MANIFEST, "global");
    assert!(
        enforcing(global, COVERAGE_FLOOR_MANIFEST),
        "the Reborn integration-coverage ratchet is no longer enforcing. It is one of the five \
         WS0 baselines (CHECKLIST WS0) and the only one guarding test coverage while suites \
         move between crates. Restore `[global].enforce = true` in {COVERAGE_FLOOR_MANIFEST}."
    );
    let floor_percent = global
        .get("floor_percent")
        .and_then(toml::Value::as_float)
        .unwrap_or_else(|| {
            panic!("{COVERAGE_FLOOR_MANIFEST} [global].floor_percent must be a float")
        });
    assert!(
        floor_percent > 0.0,
        "{COVERAGE_FLOOR_MANIFEST} [global].floor_percent must be a positive percentage, got \
         {floor_percent}"
    );

    // Visible with `-- --nocapture`; the point is that a reader can see WS0
    // and today side by side without re-deriving either number by hand.
    eprintln!("WS0 restructure baselines (measured from {WS0_MEASURED_FROM})");
    eprintln!(
        "  composition mass     : {WS0_COMPOSITION_SRC_LOC} / {WS0_COMPOSITION_DENOMINATOR_LOC} \
         LOC = {WS0_COMPOSITION_SHARE_BP} bp  [gate ceiling {ceiling_bp} bp + {tolerance_bp} tol, armed]"
    );
    eprintln!(
        "  composition dispatch : {WS0_COMPOSITION_ARC_DYN_SITES} Arc<dyn>  [gate ceiling \
         {arc_dyn_ceiling} + {arc_dyn_tolerance} tol, armed]"
    );
    eprintln!(
        "  integration coverage : floor {WS0_INTEGRATION_COVERAGE_FLOOR_PERCENT}% at WS0, \
         {floor_percent}% today  [armed]"
    );
    eprintln!(
        "  re-measure mass with : bash scripts/ci/check-composition-budget.sh --print (the gate \
         owns the metric definition; this record never re-implements it)"
    );
}
