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
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;
    // Depth, not a bool: Rust block comments nest (`/* /* … */ still comment */`),
    // so a bool would exit at the first `*/` and scan the tail as code.
    let mut depth = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if depth > 0 {
            if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                depth += 1;
                out.push(' ');
                out.push(' ');
                index += 2;
                continue;
            }
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                depth -= 1;
                out.push(' ');
                out.push(' ');
                index += 2;
                continue;
            }
            // Keep newlines so reported line numbers stay accurate.
            out.push(if byte == b'\n' { '\n' } else { ' ' });
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            depth = 1;
            out.push(' ');
            out.push(' ');
            index += 2;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                out.push(' ');
                index += 1;
            }
            continue;
        }
        out.push(byte as char);
        index += 1;
    }
    out
}

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
        for (number, line) in strip_comments(&contents).lines().enumerate() {
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
    let result_meta = root.join("crates/ironclaw_host_api/src/result_meta.rs");
    let raw = std::fs::read_to_string(&result_meta)
        .unwrap_or_else(|error| panic!("read {}: {error}", result_meta.display()));
    // Same rule as the scan above: prose explaining the retired open set is
    // worth keeping, so only code is policed.
    let contents = strip_comments(&raw);

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

/// The comment contract is a promise this gate makes to every file it scans,
/// so it gets its own coverage rather than being trusted. A guardrail whose
/// exemption rule is untested is a guardrail that silently changes meaning.
#[cfg(test)]
mod strip_comments_tests {
    use super::strip_comments;

    fn scans_as_code(source: &str, term: &str) -> bool {
        strip_comments(source).contains(term)
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
        let stripped = strip_comments("/* one\ntwo\nthree */\npub enum CapabilityErrorClass {}");
        let line = stripped
            .lines()
            .position(|line| line.contains("CapabilityErrorClass"))
            .expect("term survives on its own line");
        assert_eq!(line, 3, "the term is on source line 4 (0-indexed 3)");
    }
}
