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
//! They are now one closed `ironclaw_host_api::result_meta::FailureKind` plus projection
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

#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

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
    "DispatchError::Mcp",
    "DispatchError::Script",
    "DispatchError::Wasm",
    "DispatchError::FirstParty",
    "DispatchErrorLane",
    "dispatch_error_lane",
    "ToolError::Failed",
];

// Crate paths are spelled flat (`crates/ironclaw_x/...`) and RESOLVED through
// the crate inventory, so the family move (PROPOSAL section 5) repoints them
// without editing the literals. Identity on today's tree - pinned by
// `reborn_crate_inventory.rs` (CHECKLIST WS10).
use ratchet_support::{crate_path, strip_comments_and_strings, workspace_root};

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

/// Blank out comment spans so prose about the retired vocabulary stays legal
/// while code that names it does not, preserving line numbers for reporting.
///
/// Handles `//` line comments and `/* … */` block comments, including the
/// `/** … */` doc form and **nested** blocks (Rust permits `/* /* … */ … */`),
/// because the module contract above promises comments are exempt — a promise
/// the earlier line-only version did not keep. Covered by `strip_comments_*`
/// tests below.
///
/// Deliberately conservative about string literals: a `//` or `/*` inside one
/// blanks the rest of that span, which can only ever *hide* a hit, never
/// invent one. A retired type name inside a string literal would be
/// stringly-typed handling of the exact domain this collapse made typed, so
/// the blind spot is not worth a real parser.
/// Scan one directory tree, failing loudly on anything unreadable.
///
/// A gate that silently skips a path it cannot read is a gate that passes
/// vacuously — the failure mode this whole PR exists to remove, so it would be
/// perverse to ship it here. Every IO error propagates with its path.
fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) -> std::io::Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| std::io::Error::other(format!("read_dir {}: {error}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            std::io::Error::other(format!("entry under {}: {error}", dir.display()))
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits)?;
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
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| std::io::Error::other(format!("read {relative}: {error}")))?;
        for (number, line) in strip_comments_and_strings(&contents).lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            for term in RETIRED_TYPES.iter().chain(RETIRED_MAPPERS) {
                if line.contains(term) {
                    hits.push(format!("{relative}:{}: `{term}`", number + 1));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn reborn_code_never_redeclares_the_failure_vocabulary() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits).expect("scan crates/");
    scan_dir(&root, &root.join("tests"), &mut hits).expect("scan tests/");
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "the retired failure vocabulary is back in live code — one closed \
         `ironclaw_host_api::result_meta::FailureKind` plus projections is the single \
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
    let result_meta = crate_path(&root, "crates/ironclaw_host_api/src/result_meta.rs");
    let raw = std::fs::read_to_string(&result_meta)
        .unwrap_or_else(|error| panic!("read {}: {error}", result_meta.display()));
    // Same rule as the scan above: prose explaining the retired open set is
    // worth keeping, so only code is policed.
    let contents = strip_comments_and_strings(&raw);

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

fn assert_tool_error_rejected_uses_runtime_kind(contents: &str) {
    let rejected = contents
        .split("pub enum ToolError")
        .nth(1)
        .expect("pub enum ToolError")
        .split("Rejected {")
        .nth(1)
        .and_then(|tail| tail.split('}').next())
        .expect("ToolError::Rejected variant");

    assert!(
        rejected.contains("kind: RuntimeDispatchErrorKind"),
        "extension adapters may report runtime failures only; trusted dispatch owns control-plane kinds"
    );
    assert!(
        !rejected.contains("DispatchFailureKind"),
        "ToolError::Rejected must not accept the host's control-plane failure taxonomy"
    );
}

#[test]
fn extension_tool_errors_cannot_mint_control_plane_failure_kinds() {
    let root = workspace_root();
    let tool_adapter = crate_path(
        &root,
        "crates/ironclaw_extension_contracts/src/tool_adapter.rs",
    );
    let raw = std::fs::read_to_string(&tool_adapter)
        .unwrap_or_else(|error| panic!("read {}: {error}", tool_adapter.display()));
    let contents = strip_comments_and_strings(&raw);

    assert_tool_error_rejected_uses_runtime_kind(&contents);
}

#[test]
#[should_panic(expected = "extension adapters may report runtime failures only")]
fn extension_tool_error_scope_ratchet_rejects_malformed_source() {
    let malformed = r#"
        enum UnrelatedError {
            Rejected { kind: RuntimeDispatchErrorKind },
        }

        pub enum ToolError {
            Rejected { kind: DispatchFailureKind },
        }
    "#;

    assert_tool_error_rejected_uses_runtime_kind(malformed);
}

/// The comment contract is a promise this gate makes to every file it scans,
/// so it gets its own coverage rather than being trusted. A guardrail whose
/// exemption rule is untested is a guardrail that silently changes meaning.
#[cfg(test)]
mod strip_comments_tests {
    use super::strip_comments_and_strings;

    fn scans_as_code(source: &str, term: &str) -> bool {
        strip_comments_and_strings(source).contains(term)
    }

    #[test]
    fn line_comments_are_exempt_but_trailing_code_is_not() {
        assert!(!scans_as_code(
            "// CapabilityFailureKind was retired\nfn ok() {}",
            "CapabilityFailureKind"
        ));
        // Code BEFORE a trailing comment still scans.
        assert!(scans_as_code(
            "use CapabilityErrorClass; // retired\n",
            "CapabilityErrorClass"
        ));
    }

    #[test]
    fn block_and_doc_comments_are_exempt_across_lines() {
        assert!(!scans_as_code(
            "/*\n RuntimeFailureKind lived here\n across lines\n*/\nfn ok() {}",
            "RuntimeFailureKind"
        ));
        assert!(!scans_as_code(
            "/** doc block naming CapabilityFailureKind */\nfn ok() {}",
            "CapabilityFailureKind"
        ));
    }

    #[test]
    fn nested_block_comments_do_not_end_early() {
        // Rust block comments nest. A bool-based stripper would close at the
        // inner `*/` and scan `RuntimeFailureKind` as live code.
        assert!(!scans_as_code(
            "/* outer /* inner */ RuntimeFailureKind still commented */\nfn ok() {}",
            "RuntimeFailureKind"
        ));
    }

    #[test]
    fn code_after_a_closed_block_still_scans() {
        assert!(scans_as_code(
            "/* retired: CapabilityFailureKind */ pub enum CapabilityErrorClass {}",
            "CapabilityErrorClass"
        ));
    }

    #[test]
    fn line_numbers_survive_stripping() {
        // Hits are reported by line, so blanking must preserve newlines.
        let stripped =
            strip_comments_and_strings("/* one\ntwo\nthree */\npub enum CapabilityErrorClass {}");
        let line = stripped
            .lines()
            .position(|line| line.contains("CapabilityErrorClass"))
            .expect("term survives on its own line");
        assert_eq!(line, 3, "the term is on source line 4 (0-indexed 3)");
    }
}
