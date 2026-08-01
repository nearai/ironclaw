//! PROPOSAL §11.2.4 — port-location scan for the `Loop*Port` family.
//!
//! A port's *definition* lives in the contracts crate that owns it; its
//! *implementations* live wherever the crate that owns the behavior wires them.
//! For the loop tier that owner is `ironclaw_loop_contracts` (PROPOSAL §6.1.4,
//! `families/contracts.md`), and the rule has two halves:
//!
//! 1. **One home per port.** Every `Loop*Port` trait in the workspace is
//!    defined in exactly one crate, and that crate is the one frozen below.
//!    Adding a port means adding a row here — deliberately, in review.
//! 2. **One import path per port.** No crate re-exports another crate's
//!    `Loop*Port` trait. That is the "re-export-path trap" §11.2.4 names: two
//!    import paths for one trait let a consumer take a dependency on the wrong
//!    crate without ever naming it, which is exactly how `agent_loop`'s
//!    contracts-only rule was satisfiable on paper and violated in practice
//!    before WS1.2.
//!
//! Scope note: the predicate is anchored (`Loop…Port`), so decorator and
//! adapter type names that merely *contain* a port name — `HookedLoopModelPort`,
//! `HostQueueLoopInputPort`, `HostManagedLoopPromptPort` — are correctly out of
//! scope. They are implementations, and implementations are supposed to live
//! above the contract.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// Only `workspace_root` / `collect_type_defs` / `scan_type_defs` are reachable
// from this binary; the rest of the shared module belongs to sibling ratchets.
#[allow(dead_code)]
mod ratchet_support;

use ratchet_support::{TypeDefOccurrence, collect_type_defs, scan_type_defs, workspace_root};

/// Every `Loop*Port` trait in the workspace and the crate that owns its
/// definition.
///
/// The eleven `ironclaw_loop_contracts` rows are PROPOSAL §6.1.4's port set
/// verbatim. The two others are deliberate and documented:
///
/// * `LoopExitEvidencePort` is **kernel authority, not loop contract**. It is
///   the read-only durable-evidence port the turn kernel uses to validate a
///   `LoopExit` *claim* before any durable transition commits. §6.1.4 forbids
///   the contracts crate from holding the exit applier or its validation, and
///   this port exists only to serve them.
/// * `LoopAttachmentReadPort` is a `ironclaw_loop_host`-internal seam with a
///   single implementer inside that crate. It never crosses the loop/kernel
///   membrane, so it is not part of the contract the agent loop compiles
///   against.
const LOOP_PORT_OWNERS: &[(&str, &str)] = &[
    ("LoopCancellationPort", "ironclaw_loop_contracts"),
    ("LoopCapabilityPort", "ironclaw_loop_contracts"),
    ("LoopCheckpointPort", "ironclaw_loop_contracts"),
    ("LoopCompactionPort", "ironclaw_loop_contracts"),
    ("LoopContextPort", "ironclaw_loop_contracts"),
    ("LoopInputPort", "ironclaw_loop_contracts"),
    ("LoopModelPort", "ironclaw_loop_contracts"),
    ("LoopProgressPort", "ironclaw_loop_contracts"),
    ("LoopPromptPort", "ironclaw_loop_contracts"),
    ("LoopRunInfoPort", "ironclaw_loop_contracts"),
    ("LoopTranscriptPort", "ironclaw_loop_contracts"),
    ("LoopAttachmentReadPort", "ironclaw_loop_host"),
    ("LoopExitEvidencePort", "ironclaw_turns"),
];

const KEYWORDS: &[&str] = &["trait "];

/// Anchored on both ends: a `Loop*Port` is a port, `HookedLoopModelPort` is an
/// adapter. Only the former is location-governed.
fn is_loop_port(ident: &str) -> bool {
    ident.starts_with("Loop") && ident.ends_with("Port")
}

fn owning_crate(root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(root.join("crates"))
        .unwrap_or_else(|_| panic!("{} must live under crates/", path.display()));
    relative
        .components()
        .next()
        .expect("a scanned file must sit inside a crate directory")
        .as_os_str()
        .to_string_lossy()
        .into_owned()
}

fn render(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn loop_ports_are_defined_only_in_their_owning_crate() {
    let root = workspace_root();
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &root.join("crates"),
        KEYWORDS,
        &is_loop_port,
        &["reborn_loop_port_location_scan.rs"],
        &mut found,
    );

    assert!(
        !found.is_empty(),
        "the Loop*Port scan found no traits at all — the walk is broken, not the tree"
    );

    let owners: BTreeMap<&str, &str> = LOOP_PORT_OWNERS.iter().copied().collect();

    let mut violations = Vec::new();
    for (name, occurrences) in &found {
        let Some(expected) = owners.get(name.as_str()) else {
            violations.push(format!(
                "{name} is a new Loop*Port with no owner row. Ports are defined in \
                 ironclaw_loop_contracts unless there is a documented reason otherwise \
                 (PROPOSAL §11.2.4); add a row to LOOP_PORT_OWNERS with that reason. Defined at: {}",
                occurrences
                    .iter()
                    .map(|occ| render(&occ.path, &root))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        };
        if occurrences.len() > 1 {
            violations.push(format!(
                "{name} is defined {} times: {}. A port has exactly one definition; a second \
                 one is a shadow contract.",
                occurrences.len(),
                occurrences
                    .iter()
                    .map(|occ| render(&occ.path, &root))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for occurrence in occurrences {
            let actual = owning_crate(&root, &occurrence.path);
            if actual != *expected {
                violations.push(format!(
                    "{name} is defined in {actual} ({}) but is owned by {expected}",
                    render(&occurrence.path, &root)
                ));
            }
        }
    }

    for (name, expected) in LOOP_PORT_OWNERS {
        if !found.contains_key(*name) {
            violations.push(format!(
                "{name} is frozen as owned by {expected} but no definition was found — if it was \
                 deleted or renamed, remove or update its row here in the same change"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Loop*Port location rule violated (PROPOSAL §11.2.4):\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_crate_re_exports_another_crates_loop_port() {
    let root = workspace_root();
    let names: Vec<&str> = LOOP_PORT_OWNERS.iter().map(|(name, _)| *name).collect();
    let owners: BTreeMap<&str, &str> = LOOP_PORT_OWNERS.iter().copied().collect();

    let mut files = Vec::new();
    collect_rust_files(&root.join("crates"), &mut files);

    let mut violations = Vec::new();
    for path in files {
        if path.file_name().and_then(|name| name.to_str())
            == Some("reborn_loop_port_location_scan.rs")
        {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let crate_name = owning_crate(&root, &path);
        for (line_number, name) in cross_crate_port_re_exports(&contents, &names) {
            let owner = owners.get(name.as_str()).copied().unwrap_or("<unknown>");
            if crate_name == owner {
                continue;
            }
            violations.push(format!(
                "    {}:{line_number} re-exports {name}, which {owner} owns",
                render(&path, &root)
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "a Loop*Port must have one import path, so no crate may `pub use` a port it does not own \
         (PROPOSAL §11.2.4, the re-export-path trap):\n{}",
        violations.join("\n")
    );
}

/// Every `pub use` line that names one of `names` *and* resolves outside the
/// current crate — i.e. names another `ironclaw_*` crate explicitly. An
/// intra-crate `pub use self::module::LoopModelPort` is module plumbing and is
/// legal; `pub use ironclaw_loop_contracts::LoopModelPort` from another crate is
/// a second import path and is not.
fn cross_crate_port_re_exports(source: &str, names: &[&str]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut statement = String::new();
    let mut statement_line = 0usize;
    for (index, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if statement.is_empty() {
            if !line.starts_with("pub use ") {
                continue;
            }
            statement_line = index + 1;
        }
        statement.push(' ');
        statement.push_str(line);
        if !line.ends_with(';') {
            continue;
        }
        let finished = std::mem::take(&mut statement);
        if finished.contains("ironclaw_") {
            for name in names {
                if finished
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .any(|token| token == *name)
                {
                    out.push((statement_line, (*name).to_string()));
                }
            }
        }
    }
    out
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            // `target` alone is not enough: this walk is rooted at `crates/`,
            // which contains `ironclaw_webui/frontend`, so on any checkout
            // where the frontend has been installed the scan descends the whole
            // npm tree. Measured on this worktree: `crates/` holds 516
            // directories with the frontend *uninstalled*, and the frontend
            // declares 23 direct dependencies whose transitive closure is what
            // gets walked once `npm install` has run — pure cost, since no
            // `Loop*Port` is ever defined there. Matches the exclusion the
            // sibling scanners already use (`reborn_cross_crate_include_scan`,
            // `reborn_sealed_evidence_mint_ratchet`, …); `.git` comes along for
            // the same reason should this helper ever be rooted higher.
            let name = path.file_name().and_then(|name| name.to_str());
            if matches!(name, Some("target" | ".git" | "node_modules")) {
                continue;
            }
            collect_rust_files(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// Scanner self-tests — the guardrail must fail loudly on its own regressions
// (CHECKLIST WS10: every new scanner lands with positive + negative fixtures).
// ---------------------------------------------------------------------------

#[test]
fn loop_port_predicate_is_anchored_at_both_ends() {
    // Positive: the port family itself.
    assert!(is_loop_port("LoopModelPort"));
    assert!(is_loop_port("LoopCapabilityPort"));
    assert!(is_loop_port("LoopAttachmentReadPort"));
    // Negative: decorators and adapters that merely contain a port name. These
    // are implementations and must stay out of the location rule, or the rule
    // would forbid exactly the layering it exists to require.
    assert!(!is_loop_port("HookedLoopModelPort"));
    assert!(!is_loop_port("HostQueueLoopInputPort"));
    assert!(!is_loop_port("HostManagedLoopPromptPort"));
    // Negative: neighbours that are neither.
    assert!(!is_loop_port("LoopRunContext"));
    assert!(!is_loop_port("ProcessTransitionPort"));
}

#[test]
fn port_definition_scan_matches_definitions_not_references() {
    let sample = r##"
        pub trait LoopModelPort {}
        pub(crate) trait LoopHiddenPort {}
        trait LoopPrivatePort {}                 // not pub-visible -> ignored
        pub struct LoopModelPortAdapter;         // not a trait -> ignored
        impl LoopModelPort for Thing {}          // reference -> ignored
        let x = "pub trait LoopStringPort {}";   // string literal -> ignored
        // pub trait LoopCommentedPort {}        -> ignored
    "##;
    let got: Vec<String> = scan_type_defs(sample, KEYWORDS, &is_loop_port)
        .into_iter()
        .map(|(ident, _)| ident)
        .collect();
    assert_eq!(got, vec!["LoopModelPort", "LoopHiddenPort"]);
}

#[test]
fn re_export_scan_separates_intra_crate_plumbing_from_cross_crate_paths() {
    let names = ["LoopModelPort", "LoopCapabilityPort"];

    // Negative fixtures: legal.
    assert!(
        cross_crate_port_re_exports("pub use self::model::LoopModelPort;", &names).is_empty(),
        "intra-crate module plumbing is legal"
    );
    assert!(
        cross_crate_port_re_exports("pub use model::{LoopModelPort, Other};", &names).is_empty(),
        "an unqualified intra-crate re-export is legal"
    );
    assert!(
        cross_crate_port_re_exports("use ironclaw_loop_contracts::LoopModelPort;", &names)
            .is_empty(),
        "a plain (non-pub) import is not a second import path"
    );
    assert!(
        cross_crate_port_re_exports("// pub use ironclaw_loop_contracts::LoopModelPort;", &names)
            .is_empty(),
        "a commented-out re-export is not a re-export"
    );
    assert!(
        cross_crate_port_re_exports("pub use ironclaw_loop_contracts::LoopRunContext;", &names)
            .is_empty(),
        "re-exporting a non-port DTO is out of scope for this rule"
    );

    // Positive fixtures: the trap.
    assert_eq!(
        cross_crate_port_re_exports("pub use ironclaw_loop_contracts::LoopModelPort;", &names),
        vec![(1, "LoopModelPort".to_string())]
    );
    assert_eq!(
        cross_crate_port_re_exports(
            "pub use ironclaw_loop_contracts::{\n    LoopCapabilityPort,\n    Other,\n};",
            &names
        ),
        vec![(1, "LoopCapabilityPort".to_string())],
        "a multi-line group re-export is still one statement"
    );
}
