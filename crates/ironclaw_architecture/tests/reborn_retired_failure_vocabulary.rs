//! Zero-legacy gate for the unified failure vocabulary (#6284).
//!
//! Five enums used to answer one question — *why did this operation fail* —
//! each re-declaring the domain at a different altitude:
//!
//! | retired | crate | variants |
//! |---|---|---|
//! | `FailureKind` (open-set shape) | `ironclaw_host_api` | 19 |
//! | `CapabilityFailureKind` | `ironclaw_turns` | 19 (identical names) |
//! | `RuntimeFailureKind` | `ironclaw_host_runtime` | 17 (strict subset) |
//! | `CapabilityErrorClass` | `ironclaw_agent_loop` | 7 |
//!
//! Re-declared domains drift, and the drift is where recoverability died: the
//! fold from the 22 mechanism-precise dispatch names down to 12 coarse ones
//! destroyed 17 names — and with them every remediation hint the model could
//! have acted on.
//!
//! They are now one closed `ironclaw_host_api::FailureKind` plus projection
//! functions (`fate()`, retry-category, wire tag, HTTP status). Every
//! projection is a wildcard-free exhaustive match beside the single
//! definition, so a new kind cannot compile until each consumer classifies it.
//!
//! This test pins the retired names at **zero live occurrences** so the
//! collapse cannot quietly reverse — a cleanup nobody enforces is a cleanup
//! that comes back.
//!
//! **Comments are exempt on purpose.** Doc comments that explain what was
//! retired and why are worth keeping; only code is policed. The scan therefore
//! skips comment lines rather than path-scoping the files that carry that
//! history.

use std::path::{Path, PathBuf};

/// Type names retired by the collapse. Any live reference means a layer has
/// started re-declaring the failure domain again.
const RETIRED_TYPES: &[&str] = &[
    "CapabilityFailureKind",
    "RuntimeFailureKind",
    "CapabilityErrorClass",
    // The open-set escape hatches. The vocabulary is deliberately closed: an
    // unnameable failure is `FailureKind::Unclassified` (model-visible, never
    // retried), not free text that lets a producer skip classifying.
    "CapabilityFailureKindValue",
    "FailureKindValue",
];

/// Conversion helpers that existed only to move a value between two spellings
/// of the same domain. Their absence is what proves the collapse is real
/// rather than a rename with the mapping tax still attached.
const RETIRED_MAPPERS: &[&str] = &[
    "runtime_failure_kind_to_loop",
    "model_visible_runtime_failure_kind_to_loop",
    "capability_error_class",
    "capability_failure_kind_from",
    "failure_kind_of",
    "exhausted_capability_failure_kind",
];

fn workspace_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/ironclaw_architecture -> crates -> workspace root
    path.pop();
    path.pop();
    path
}

/// Strip line comments so prose about the retired vocabulary stays legal while
/// code that names it does not. Deliberately conservative: a `//` inside a
/// string literal ends the line early, which can only ever *hide* a hit on
/// that line, never invent one. String literals naming these types would be
/// stringly-typed handling of the exact domain this collapse made typed, so
/// the blind spot is not one worth widening the parser for.
fn code_only(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("#!") {
        return "";
    }
    match line.find("//") {
        Some(index) => &line[..index],
        None => line,
    }
}

fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits);
            continue;
        }
        if !name.ends_with(".rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // This test names every retired term on purpose.
        if relative.ends_with("reborn_retired_failure_vocabulary.rs") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in contents.lines().enumerate() {
            let code = code_only(line);
            if code.is_empty() {
                continue;
            }
            for term in RETIRED_TYPES.iter().chain(RETIRED_MAPPERS) {
                if code.contains(term) {
                    hits.push(format!("{relative}:{}: `{term}`", number + 1));
                }
            }
        }
    }
}

#[test]
fn reborn_code_never_redeclares_the_failure_vocabulary() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits);
    scan_dir(&root, &root.join("tests"), &mut hits);
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "the retired failure vocabulary is back in live code — one closed \
         `ironclaw_host_api::FailureKind` plus projections is the single \
         definition (#6284); a second spelling drifts from it, and the drift \
         is where recoverability dies:\n{}",
        hits.join("\n")
    );
}

/// The gate above is only meaningful if the survivor is genuinely closed —
/// a reintroduced open-set escape hatch would let a producer skip
/// classification without ever naming a retired type.
#[test]
fn the_surviving_failure_vocabulary_stays_closed() {
    let root = workspace_root();
    let result_meta = root.join("crates/ironclaw_host_api/src/result_meta.rs");
    let raw = std::fs::read_to_string(&result_meta)
        .unwrap_or_else(|error| panic!("read {}: {error}", result_meta.display()));
    // Same rule as the scan above: prose explaining the retired open set is
    // worth keeping, so only code is policed.
    let contents: String = raw.lines().map(code_only).collect::<Vec<_>>().join("\n");

    assert!(
        contents.contains("Unclassified"),
        "`FailureKind::Unclassified` is the one sanctioned sink for a failure \
         the system cannot name — model-visible and never retried. Removing it \
         forces producers back onto an open-set escape hatch."
    );
    assert!(
        !contents.contains("Unknown(") && !contents.contains("unknown("),
        "`FailureKind` must stay closed: no payload-carrying open-set variant. \
         An unnameable failure is `Unclassified`, not free text."
    );
    assert!(
        !contents.contains("#[non_exhaustive]"),
        "`FailureKind` is deliberately NOT `#[non_exhaustive]` — downstream \
         projections match exhaustively so a new variant fails to compile until \
         it is classified. That compile error is the recoverability review."
    );
}
