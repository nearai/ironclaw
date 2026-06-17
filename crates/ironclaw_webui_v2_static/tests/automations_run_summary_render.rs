//! Source-shape lock for the recent-run summary (#4988).
//!
//! The summary chips are rendered by `RunHistorySummary` in
//! `automation-recent-runs.js`. The status-bucket decision (which chips, their
//! localized text, the total) lives in the pure `runSummaryView` presenter,
//! which has behavioral coverage in `automations-presenters.test.mjs`
//! (`runSummaryView renders the unknown chip and chips sum to total`).
//!
//! This crate has no browser/React test harness — React is loaded from esm.sh
//! via an import map at runtime and CI only `node --check`s JS syntax, so the
//! component itself cannot be rendered in a unit test. We therefore lock the
//! *source shape* of the caller (the same technique the composition crate uses
//! for its `static_*` asset tests): `RunHistorySummary` must render the whole
//! `view.chips` list 1:1 and must not re-introduce any per-status filtering or
//! allow-listing that could silently drop a counted bucket (e.g. `unknown`).
//! If a future edit adds `view.chips.filter(...)` or maps only known keys, this
//! test fails — covering the regression a helper-only test cannot.
//!
//! Pure file parsing — no dependency on the crate's `webui-v2-beta` API, so it
//! runs under the default feature set in CI.

use std::fs;
use std::path::PathBuf;

fn read_static(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("static/js")
        .join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn read_dist(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("static/dist")
        .join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn count_matches(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

/// Extract the body of an exported function by brace-matching from its
/// `export function <name>(` declaration. Panics if the function is missing or
/// unbalanced so a rename can't silently void the assertions below.
fn export_function_body(src: &str, name: &str) -> String {
    let needle = format!("export function {name}(");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("`{needle}` not found"));
    let bytes = src.as_bytes();

    // Skip the parameter list (which may contain `{}` destructuring) by
    // paren-matching from the first `(`, then take the function body brace.
    let params_open = start + needle.len() - 1;
    let mut paren_depth = 0usize;
    let mut params_close = None;
    for (i, &b) in bytes.iter().enumerate().skip(params_open) {
        match b {
            b'(' => paren_depth += 1,
            b')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    params_close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let params_close = params_close.unwrap_or_else(|| panic!("unbalanced params in `{name}`"));
    let open = src[params_close..]
        .find('{')
        .map(|i| params_close + i)
        .unwrap_or_else(|| panic!("no opening brace after `{needle}` params"));
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return src[open..=i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces in `{name}`");
}

#[test]
fn run_history_summary_renders_every_chip_without_filtering() {
    let src = read_static("pages/automations/components/automation-recent-runs.js");
    let body = export_function_body(&src, "RunHistorySummary");

    // Data comes from the tested presenter, not inline status logic.
    assert!(
        body.contains("runSummaryView("),
        "RunHistorySummary must derive its data from runSummaryView; \
         inline status logic would not be covered by the presenter test"
    );

    // The chips are rendered 1:1 from the resolved view.
    assert!(
        body.contains("view.chips.map("),
        "RunHistorySummary must render the whole view.chips list"
    );

    // No per-status filtering / allow-listing that could drop a counted bucket
    // (the exact regression vector: filter view.chips, or map only known keys).
    assert!(
        !body.contains("view.chips.filter("),
        "RunHistorySummary must not filter view.chips — that can silently drop \
         a counted status bucket (e.g. unknown). Filtering belongs in the \
         tested presenter, not the renderer."
    );
    assert!(
        !body.contains(".filter("),
        "RunHistorySummary must not filter the rendered chips; render all of them"
    );
}

#[test]
fn run_dots_does_not_duplicate_empty_run_copy() {
    let src = read_static("pages/automations/components/automation-recent-runs.js");
    let body = export_function_body(&src, "RunDots");

    assert!(
        body.contains("return null;"),
        "RunDots should render nothing for an empty run list; RunHistorySummary \
         owns the single 'No runs' empty-state copy when both components are \
         rendered together"
    );
    assert!(
        !body.contains("automations.table.noRuns"),
        "RunDots must not render the no-runs label, or the automations table \
         shows duplicate empty-state text"
    );
}

#[test]
fn run_dots_marks_error_runs_warning() {
    let src = read_static("pages/automations/components/automation-recent-runs.js");
    let body = export_function_body(&src, "RunDots");

    assert!(
        body.contains("run.status === \"error\"") && body.contains("bg-[var(--v2-warning-text)]"),
        "error recent-run dots should use the warning attention treatment"
    );
    assert!(
        !body.contains("run.status === \"error\" && \"border-red"),
        "error recent-run dots must not regress to the red terminal-error styling"
    );
}

#[test]
fn committed_bundle_matches_recent_run_empty_and_attention_treatment() {
    let bundle = read_dist("app.js");

    assert_eq!(
        count_matches(&bundle, "automations.table.noRuns"),
        2,
        "the served bundle should contain the no-runs i18n entry plus the \
         RunHistorySummary renderer only; an extra occurrence means RunDots \
         still duplicates the empty-state label in committed dist output"
    );
    assert!(
        bundle.contains("bg-[var(--v2-warning-text)]"),
        "the served bundle should include the warning recent-run attention dot"
    );
    assert!(
        !bundle.contains("border-red-300/50 bg-red-400"),
        "the served bundle must not keep the old red error-dot treatment"
    );
    assert!(
        !bundle.contains("\"automations.runStatus.error\":\"Error\""),
        "automation run-status errors should not render with literal Error copy"
    );
    assert!(
        bundle.contains("\"automations.filter.failures\":\"Needs attention\"")
            && bundle.contains("\"automations.summary.failures\":\"Needs attention\"")
            && bundle.contains("\"automations.badge.warning\":\"Warning\""),
        "the served bundle should use Needs attention / warning copy for the \
         automation attention filter and summary card"
    );
}

#[test]
fn automation_summary_failure_card_uses_warning_tone() {
    let src = read_static("pages/automations/components/automations-summary-strip.js");
    let body = export_function_body(&src, "AutomationsSummaryStrip");

    assert!(
        body.contains("key: \"failures\""),
        "summary strip must still include the failures/attention card"
    );
    assert!(
        body.contains("tone: (summary?.failures ?? 0) > 0 ? \"warning\" : \"success\""),
        "failure summary card should render as warning when failures need attention"
    );
    assert!(
        body.contains("label: t(\"automations.summary.failures\")"),
        "failure summary card label should stay localized through the summary key"
    );
}

#[test]
fn automations_list_failed_rows_use_needs_review_warning_pill() {
    let src = read_static("pages/automations/components/automations-list.js");
    let body = export_function_body(&src, "AutomationsList");

    assert!(
        body.contains("automation.has_failed_runs")
            && body.contains("? \"warning\"")
            && body.contains("t(\"automations.status.needsReview\")"),
        "automations with failed recent runs should render the warning Needs attention pill"
    );
}

#[test]
fn automation_detail_success_rate_uses_warning_for_failed_runs() {
    let src = read_static("pages/automations/components/automation-detail-panel.js");
    let body = export_function_body(&src, "AutomationDetailPanel");

    assert!(
        body.contains("tone=${automation.has_failed_runs ? \"warning\" : \"success\"}"),
        "detail-panel success rate should render warning when recent runs need attention"
    );
}

#[test]
fn run_summary_presenter_includes_every_status_bucket() {
    let src = read_static("pages/automations/lib/automations-presenters.js");
    let body = export_function_body(&src, "runStatusBreakdown");

    // Every status the summarizer counts must be representable as a chip, so the
    // breakdown can never omit one. summarizeRuns buckets are ok/error/running/
    // unknown; assert all four appear here.
    for key in ["\"ok\"", "\"error\"", "\"running\"", "\"unknown\""] {
        assert!(
            body.contains(key),
            "runStatusBreakdown must include the {key} bucket so it is never \
             dropped from the rendered summary"
        );
    }

    // The view the component renders is built from the full breakdown.
    let view = export_function_body(&src, "runSummaryView");
    assert!(
        view.contains("runStatusBreakdown("),
        "runSummaryView must build its chips from runStatusBreakdown"
    );
}
