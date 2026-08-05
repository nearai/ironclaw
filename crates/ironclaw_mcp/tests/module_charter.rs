//! The `lib.rs` module-charter table is a contract, not a comment.
//!
//! Its load-bearing clause is **"no module builds a failure string of its
//! own"**: reasons come from `diagnostics`' cause enums, so every token the
//! model can see is bounded and enumerable in one file. That clause was
//! documented in three places (`src/lib.rs`, `CLAUDE.md`, and the
//! `diagnostics` module doc) and enforced in none — and the crate carried a
//! live exception, `egress.rs`, which minted `"runtime_http_egress_panicked"`
//! inline and forwarded `stable_runtime_reason()` verbatim into
//! `McpClientError`.
//!
//! This gate arms the clause for the two error taxonomies that reach a caller
//! through the client seam — `McpClientError` and `McpHostHttpError` — and
//! records, as an enumerated carve-out, the one place that still mints its own:
//! `McpError`'s descriptor/invocation reasons in `runtime.rs`. The carve-out is
//! a list, not a wildcard, so a *new* inline reason fails here even though the
//! two existing ones are grandfathered.

use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The module that owns every reason token. It is the one file allowed to
/// write a reason literal.
const NAMING_MODULE: &str = "diagnostics.rs";

/// Reason literals that predate the rule and are deliberately left in place.
///
/// Both are `McpError` variants (the *runtime* taxonomy, not the client seam)
/// carrying a caller-supplied descriptor/transport name rather than a
/// classification token. Moving them is a separate change with a real
/// behavioural surface — the tokens are asserted on downstream — so they are
/// enumerated rather than silently tolerated. Removing a row here is always
/// allowed; adding one needs the same argument these two carry.
const GRANDFATHERED_INLINE_REASONS: &[(&str, &str)] = &[
    (
        "runtime.rs",
        "McpError::DescriptorMismatch — echoes the manifest's own descriptor id/provider",
    ),
    (
        "runtime.rs",
        "McpError::InvalidInvocation — echoes the manifest's own transport name",
    ),
];

/// Every `.rs` file directly under `src/`, minus the naming module itself.
fn charted_modules() -> Vec<(String, String)> {
    let mut modules = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(src_dir())
        .expect("read src/")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    paths.sort();
    for path in paths {
        let name = file_name(&path);
        if name == NAMING_MODULE || name == "lib.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        modules.push((name, strip_test_module(&source)));
    }
    assert!(
        modules.len() >= 5,
        "expected the charter's private modules under src/; found {} — this gate \
         would be scanning nothing",
        modules.len()
    );
    modules
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("file name")
        .to_string_lossy()
        .to_string()
}

/// Drop everything from the first `#[cfg(test)]` line onward.
///
/// Test fixtures legitimately construct errors with literal reasons; the rule
/// is about production token minting.
fn strip_test_module(source: &str) -> String {
    match source.find("#[cfg(test)]") {
        Some(index) => source[..index].to_string(),
        None => source.to_string(),
    }
}

/// No module outside `diagnostics` mints a reason string of its own.
///
/// The probe is the literal shape a minted reason takes in this crate: a
/// `reason:` struct-field initialiser whose value is a string literal or a
/// `format!`. A reason that came from `diagnostics` is a function call
/// (`request_denied(…)`, `response_error(…)`, `egress_failure(…)`) or a moved
/// binding, and matches neither.
#[test]
fn no_module_outside_diagnostics_builds_a_failure_string() {
    let mut minted: Vec<String> = Vec::new();
    for (module, source) in charted_modules() {
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            let mints = trimmed.starts_with("reason: \"")
                || trimmed.starts_with("reason: format!")
                || trimmed == "reason: format!(";
            if mints {
                minted.push(format!("{module}:{}: {}", index + 1, trimmed));
            }
        }
    }

    assert_eq!(
        minted.len(),
        GRANDFATHERED_INLINE_REASONS.len(),
        "the crate charter says no module builds a failure string of its own; \
         `{NAMING_MODULE}` is the only module allowed to write a reason literal. \
         Classify the failure as a `diagnostics` cause enum and name it there. \
         Grandfathered rows ({}): {:?}\nFound {} inline reason(s):\n{}",
        GRANDFATHERED_INLINE_REASONS.len(),
        GRANDFATHERED_INLINE_REASONS,
        minted.len(),
        minted.join("\n")
    );

    // The grandfathered rows are all in one module; pin that too, so a new
    // inline reason elsewhere cannot pass by coincidence of count.
    for entry in &minted {
        assert!(
            entry.starts_with("runtime.rs:"),
            "the only grandfathered inline reasons live in runtime.rs; this one \
             does not: {entry}"
        );
    }
}

/// `McpClientError` cannot be built from a bare `String` by conversion.
///
/// `impl From<String>` made every `?` and `.into()` in the crate a silent
/// bypass of `diagnostics`. It had no caller, so it was removed; this keeps it
/// removed.
#[test]
fn mcp_client_error_has_no_blanket_string_conversion() {
    let contract =
        std::fs::read_to_string(src_dir().join("contract.rs")).expect("read contract.rs");
    assert!(
        !contract.contains("impl From<String> for McpClientError"),
        "`impl From<String> for McpClientError` re-opens the bypass the charter's \
         'no module builds a failure string of its own' rule closes: any module \
         could then turn an arbitrary String into a model-visible reason with \
         `?`. Route the failure through a `diagnostics` cause enum instead."
    );
}

/// The charter text and this gate must describe the same rule.
///
/// The failure this pins is documentation drift in the direction that matters:
/// three files promising an invariant that nothing checks.
#[test]
fn the_charter_text_names_the_rule_this_gate_enforces() {
    let lib = std::fs::read_to_string(src_dir().join("lib.rs")).expect("read lib.rs");
    let guardrails =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("CLAUDE.md"))
            .expect("read CLAUDE.md");

    for (label, text) in [("lib.rs", &lib), ("CLAUDE.md", &guardrails)] {
        // Whitespace-normalized: both files wrap the sentence, and `lib.rs`
        // prefixes every line with `//!`. A gate that depended on the wrap
        // column would fail on a reflow and teach people to delete it.
        let flat = text
            .replace("//!", " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        assert!(
            flat.contains("no module builds a failure string of its own"),
            "{label} must still state the rule this gate enforces"
        );
        assert!(
            flat.contains("module_charter.rs"),
            "{label} must name `tests/module_charter.rs` as the enforcement point, \
             so the rule and its gate are found together"
        );
    }
}
