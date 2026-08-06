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
//! cargo test -p ironclaw_architecture_tests --test reborn_restructure_baselines -- --nocapture
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
/// The metric is production `.rs` LOC of `crates/ironclaw_composition/src`
/// over the same measure of every `crates/*/src` tree (test-only files excluded
/// from both sides — see `scripts/ci/composition-budget.toml` for the full
/// definition). CHECKLIST WS6 re-baselines the gate's ceiling once the eviction
/// inventory lands; this record is what "before" was.
const WS0_COMPOSITION_SRC_LOC: usize = 43_936;
const WS0_COMPOSITION_DENOMINATOR_LOC: usize = 667_978;
const WS0_COMPOSITION_SHARE_BP: usize = 658;

/// Composition's **absolute** production LOC on `origin/main` @ `676d86ce02`
/// (2026-08-04), from the same `--print` run: "composition absolute: 44021 LOC
/// (production src)".
///
/// ✎ **Added 2026-08-04 (#7151).** The share metric above is *share-based*, and
/// that denominator is poisoned: it is every other crate's production code, so
/// feature inflow anywhere else improves composition's score while composition
/// grows. Measured across the two days before this record, composition took
/// **+619 lines** of feature inflow against **−23** from an entire eviction
/// wave — and its share still *fell*, 658 bp → 634 bp, because the workspace
/// grew faster. The share ceiling could not object either: WS0's own note
/// records ~17.4pp of slack (2398 bp ceiling against 634 bp observed), i.e.
/// composition could roughly quadruple untouched. `[gate].loc_ceiling` is the
/// bound that actually constrains, and the assertion below keeps it armed and
/// consistent the same way the share ceiling is kept.
///
/// ✎ Re-recorded 44_021 → 44_392 on 2026-08-04 with the manifest's matching
/// re-seed: #7062 landed +371 production LOC of composition wiring between
/// this record's seeding and this branch's merge of `main` @ `be33ae138f`,
/// and the nudge-window assertion below correctly refused a ceiling that
/// moved without its record (371 > 200). Measured on the merged tree with
/// `bash scripts/ci/check-composition-budget.sh --print`.
///
/// ✎ Re-recorded 45_127 → 42_938 on 2026-08-04 by the WS6 service-cluster
/// eviction: the admin-user directory and blocked-auth resume fan-out moved to
/// `ironclaw_assistant`, and turn-end trace capture split into
/// `ironclaw_trace_commons::capture` + `ironclaw_turn_runner::trace_capture`.
/// **−2,189 LOC**, and the share metric's blindness shows again in the same
/// run: 654 bp → 622 bp is a 32 bp move for a 4.9% absolute cut, because the
/// denominator barely noticed. The manifest's `loc_ceiling`/`loc_observed` are
/// lowered to match in the same commit, which is the obligation the nudge
/// assertion below exists to make visible.
/// ✎ Re-recorded 45_127 → 42_688 on 2026-08-04 by the WS6 policy evictions
/// (the profile approval gate to `ironclaw_approvals`, fire-time trigger
/// access to `ironclaw_triggers`): −2,439 production LOC, banked as the new
/// floor in the same PR that removed them, with `[gate].loc_ceiling` lowered
/// to match. Measured with `bash scripts/ci/check-composition-budget.sh
/// --print`, not derived by subtracting the diff.
/// ✎ Union re-record 2026-08-04: the two WS6 evictions above are disjoint and
/// their deltas add exactly on the merged batch — 45_127 − 2_189 − 2_439 = 40_499.
/// ✎ Re-recorded 40_499 → 40_405 on 2026-08-05 by the WS6 OpenAI-compat
/// eviction (the re-scoped clause: `openai_compat_serve.rs`'s router-state
/// assembly, `/v1/models` catalog + error map, projection streamer + envelope
/// decode, and scope→caller projection move to
/// `ironclaw_openai_compat::mount`). Measured on the post-eviction tree with
/// `bash scripts/ci/check-composition-budget.sh --print`; the same command on
/// this branch's base read **40_595**, so the −94 recorded here is a −190 net
/// eviction (−199 moved out, +9 back as the module doc-comment recording what
/// stayed and why) against a base that had drifted +96 over the old ceiling
/// and was passing on tolerance alone. `[gate].loc_ceiling`/`loc_observed`
/// move in the same commit, which is what the nudge assertion below exists to
/// force.
/// ✎ Union re-measure 2026-08-05 (tail batch): 40_405 + 1 — a WS8 consumer
/// repoint added one line in composition; recorded at the measured figure.
const COMPOSITION_ABSOLUTE_SRC_LOC: usize = 40_419;

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
    // `integer` panics on a missing key, which is the point: deleting the
    // absolute-mass keys must be a loud Rust-side failure as well as a shell
    // schema error, so the binding metric cannot be disarmed by three
    // deletions in a TOML file (#7151).
    let loc_ceiling = integer(gate, COMPOSITION_BUDGET_MANIFEST, "loc_ceiling");
    let loc_tolerance = integer(gate, COMPOSITION_BUDGET_MANIFEST, "loc_tolerance");
    let loc_nudge_slack = integer(gate, COMPOSITION_BUDGET_MANIFEST, "loc_nudge_slack");

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

    // The absolute-mass bound (#7151). Two properties, both of which the share
    // ceiling above visibly lacks.
    assert!(
        loc_ceiling > 0,
        "{COMPOSITION_BUDGET_MANIFEST} [gate].loc_ceiling is {loc_ceiling} — a zero or negative \
         absolute ceiling is a disarmed gate, not a bound. It is the only composition metric \
         that feature inflow elsewhere in the workspace cannot loosen (#7151)."
    );
    let effective_loc_ceiling = loc_ceiling + loc_tolerance;
    assert!(
        i64::try_from(COMPOSITION_ABSOLUTE_SRC_LOC).is_ok_and(|loc| loc <= effective_loc_ceiling),
        "the composition absolute-mass record ({COMPOSITION_ABSOLUTE_SRC_LOC} LOC) now exceeds \
         the gate's effective ceiling ({effective_loc_ceiling} LOC). Re-measure with `bash \
         scripts/ci/check-composition-budget.sh --print` and update this record, or the ceiling \
         is wrong."
    );
    // The ceiling has to BIND, which is the specific failure the share metric
    // suffered: 2398 bp recorded against 634 bp observed is ~17.4pp of slack,
    // enough for composition to roughly quadruple with the gate green. A
    // recorded value more than one nudge-window below the ceiling means a wave
    // closed without re-ratcheting.
    assert!(
        i64::try_from(COMPOSITION_ABSOLUTE_SRC_LOC)
            .is_ok_and(|loc| loc_ceiling - loc <= loc_nudge_slack),
        "{COMPOSITION_BUDGET_MANIFEST} [gate].loc_ceiling is {loc_ceiling} against a recorded \
         {COMPOSITION_ABSOLUTE_SRC_LOC} LOC — {} LOC of unclaimed headroom, more than the \
         {loc_nudge_slack}-LOC nudge window. Composition may grow that far with every gate \
         green, which is exactly how the share ceiling became inert. Lower loc_ceiling to the \
         observed count (re-ratchet at every wave close) or raise loc_nudge_slack with a \
         rationale.",
        loc_ceiling - COMPOSITION_ABSOLUTE_SRC_LOC as i64
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
        "  composition absolute : {COMPOSITION_ABSOLUTE_SRC_LOC} LOC  [gate ceiling \
         {loc_ceiling} + {loc_tolerance} tol, nudge at {loc_nudge_slack}, armed]  <- the \
         binding mass bound"
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
