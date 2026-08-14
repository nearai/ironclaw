//! The Web Debug Inspector's turn-navigation window must not exceed what the
//! host actually retains.
//!
//! The browser remembers observed run ids per thread and offers previous/next
//! navigation across them; the host keeps bounded diagnostics for only the most
//! recent `DEFAULT_MAX_RETAINED_RUNS_PER_SESSION` runs of a
//! `(tenant, user, thread)` session. When the browser window was wider than the
//! host's retention, navigation advertised turns whose snapshot came back empty
//! — a silent dead zone no per-layer test could see, because each side was
//! individually correct and the e2e scenario happened to stop at the limit.
//!
//! The two constants live in different languages and cannot import each other,
//! so this gate is the only thing holding them together.

use std::fs;

use ironclaw_product_contracts::inspector::DEFAULT_MAX_RETAINED_RUNS_PER_SESSION;

mod ratchet_support;

const BROWSER_CONSTANT: &str = "MAX_INSPECTOR_RUNS_PER_THREAD";
/// Logical, family-independent spelling resolved through
/// `ratchet_support::crate_path`, so this gate survives a crate family move.
const BROWSER_SOURCE: &str =
    "crates/ironclaw_webui/frontend/src/pages/chat/inspector/inspector-activity.ts";

/// Read `export const MAX_INSPECTOR_RUNS_PER_THREAD = <n>;` from the SPA source.
fn browser_turn_window(source: &str) -> Option<usize> {
    let declaration = source
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("export const") && line.contains(BROWSER_CONSTANT))?;
    declaration
        .split('=')
        .nth(1)?
        .trim()
        .trim_end_matches(';')
        .trim()
        .replace('_', "")
        .parse()
        .ok()
}

#[test]
fn reborn_inspector_retention_alignment() {
    let root = ratchet_support::workspace_root();
    let path = ratchet_support::crate_path(&root, BROWSER_SOURCE);
    let source = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "read {}: {error}. The inspector turn-navigation window is pinned to \
             DEFAULT_MAX_RETAINED_RUNS_PER_SESSION here; if the SPA file moved within \
             the crate, update BROWSER_SOURCE rather than dropping the check.",
            path.display()
        )
    });

    let browser_window = browser_turn_window(&source).unwrap_or_else(|| {
        panic!(
            "could not read `{BROWSER_CONSTANT}` from {}. It must stay an exported \
             plain numeric literal so this alignment gate can read it.",
            path.display()
        )
    });

    assert!(
        browser_window <= DEFAULT_MAX_RETAINED_RUNS_PER_SESSION,
        "inspector turn navigation offers {browser_window} turns per thread but the host \
         retains diagnostics for only {DEFAULT_MAX_RETAINED_RUNS_PER_SESSION} runs per \
         session, so turn {} and older would render blank. Either lower \
         `{BROWSER_CONSTANT}` in {} or raise \
         `DEFAULT_MAX_RETAINED_RUNS_PER_SESSION` — and if you raise it, say what the \
         added resident memory buys, because diagnostic capture is unconditional.",
        DEFAULT_MAX_RETAINED_RUNS_PER_SESSION + 1,
        path.display(),
    );
}
