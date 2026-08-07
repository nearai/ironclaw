//! Zero-legacy gate for the memory lifecycle-capabilities rework (#3537).
//!
//! The rework retired a vocabulary: the `[memory]` operation families
//! (`MemoryOperationKind` / `operations = [...]` / the mandatory
//! `document_store` family), the single dual-lane `retrieve_context` provider
//! method (replaced by explicit `read_long_term` / `read_short_term` lane
//! methods), and the Rust-declared `builtin.profile_set` capability (now
//! `ironclaw.memory.profile_set`, declared by the bound provider's manifest).
//! This test pins all of it at **zero occurrences** across Reborn code
//! (`crates/`, including the WebUI frontend sources, and
//! `tests/integration/`) so none of it can be reintroduced silently — same
//! shape as `reborn_retired_taxonomy.rs`.
//!
//! Sanctioned exceptions are path-scoped, not term-scoped: this test names
//! every term on purpose. The list is shrink-only and pinned to reality — see
//! `SANCTIONED_PATHS`.

#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::workspace_root;

/// Retired memory vocabulary. A hit outside the sanctioned paths is a
/// regression, not a style issue.
const RETIRED_TERMS: &[&str] = &[
    // The operation-family enum (replaced by MemoryLifecycleHook).
    "MemoryOperationKind",
    // The dual-lane provider method (replaced by the explicit lane methods).
    "retrieve_context",
    // The `[memory].operations` manifest key (replaced by `lifecycle = [...]`
    // plus the provider's own `[[tools]]`).
    "operations = [",
    // The Rust-declared builtin profile tool (now the provider-declared
    // `ironclaw.memory.profile_set`).
    "builtin.profile_set",
    // The mandatory operation family (the `[[tools]]` array IS the
    // document-tool surface; no enum variant may mean "assume four tools").
    "document_store",
];

/// Path fragments allowed to reference retired vocabulary.
///
/// Shrink-only, and pinned to reality by
/// `sanctioned_paths_each_resolve_to_exactly_one_file`: a fragment matching no
/// scanned file exempts nothing, so it is dead text that reads as policy, and a
/// fragment matching *more than one* exempts more than it says. The
/// `crates/ironclaw_gateway/` entry had already outlived its crate when that
/// check was added — invisibly, because a fragment that matches nothing simply
/// never sanctions anything.
const SANCTIONED_PATHS: &[&str] = &[
    // This gate names every term on purpose.
    "reborn_memory_retired_vocabulary.rs",
];

/// Sanity floor for the scan, mirrored from `reborn_retired_taxonomy.rs` (this
/// gate's twin — which had the floor from birth while this file did not). An
/// *unreadable* path already fails the walk outright, so what the floor guards
/// is the shape no I/O error can reveal: a `crates/` that exists and holds a
/// fraction of the tree — the partial family move — where "no retired
/// vocabulary found" is indistinguishable from "almost nothing was looked at"
/// (CHECKLIST WS0, #6963). Far below the real count (~4000) and never expected
/// to bind.
const MIN_SCANNED_FILES: usize = 500;

fn is_sanctioned(path: &str) -> bool {
    SANCTIONED_PATHS
        .iter()
        .any(|fragment| path.contains(fragment))
}

/// A scan error is a gate failure, not a skip: an unreadable directory or
/// file could hide a reintroduced term.
fn scan_dir(
    root: &Path,
    dir: &Path,
    hits: &mut Vec<String>,
    scanned: &mut Vec<String>,
) -> std::io::Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits, scanned)?;
            continue;
        }
        let is_rust = name.ends_with(".rs");
        let is_frontend = name.ends_with(".ts")
            || name.ends_with(".tsx")
            || name.ends_with(".mts")
            || name.ends_with(".mjs")
            || name.ends_with(".js");
        let is_manifest = name.ends_with(".toml");
        if !(is_rust || is_frontend || is_manifest) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        scanned.push(relative.clone());
        if is_sanctioned(&relative) {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| std::io::Error::new(error.kind(), format!("{relative}: {error}")))?;
        for term in RETIRED_TERMS {
            if contents.contains(term) {
                hits.push(format!("{relative}: `{term}`"));
            }
        }
    }
    Ok(())
}

/// Every scanned path, plus the hits. A scan error is a gate failure, not a
/// skip, so both walks are `expect`ed rather than tolerated.
fn scan_workspace(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut hits = Vec::new();
    let mut scanned = Vec::new();
    scan_dir(root, &root.join("crates"), &mut hits, &mut scanned)
        .expect("scan crates/ without I/O errors");
    scan_dir(
        root,
        &root.join("tests/integration"),
        &mut hits,
        &mut scanned,
    )
    .expect("scan tests/integration without I/O errors");
    hits.sort();
    hits.dedup();
    (hits, scanned)
}

/// Which scanned file a sanctioned fragment resolves to, refusing both the
/// stale and the ambiguous case.
///
/// `is_sanctioned` matches with `contains`, so a fragment is only as precise as
/// the tree makes it: a second file whose path contains `"…retired_vocabulary.rs"`
/// would be exempted too, silently and invisibly. Requiring exactly one match
/// keeps the fragment form (a workspace-relative literal would re-key this list
/// to the flat `crates/<name>/` depth this PR exists to remove) while making
/// ambiguity a failure rather than a widening.
fn resolve_sanctioned<'a>(fragment: &str, scanned: &'a [String]) -> Result<&'a str, String> {
    let matches: Vec<&str> = scanned
        .iter()
        .map(String::as_str)
        .filter(|path| path.contains(fragment))
        .collect();
    match matches.as_slice() {
        [single] => Ok(single),
        [] => Err(format!(
            "`{fragment}` matches no scanned file — delete it; this list is shrink-only and \
             an entry that sanctions nothing is dead text that reads as policy"
        )),
        _ => Err(format!(
            "`{fragment}` matches {} scanned files ({matches:?}) — an exemption must name \
             exactly one file, or it silently exempts every future file whose path happens \
             to contain the fragment. Lengthen the fragment until it is unambiguous",
            matches.len()
        )),
    }
}

/// An exemption that matches nothing exempts nothing, and one that matches more
/// than one file exempts more than it says. Mirrors the stale-entry check in
/// `reborn_retired_taxonomy.rs`, this gate's twin, and adds the ambiguity half.
#[test]
fn sanctioned_paths_each_resolve_to_exactly_one_file() {
    let (_, scanned) = scan_workspace(&workspace_root());
    let problems: Vec<String> = SANCTIONED_PATHS
        .iter()
        .filter_map(|fragment| resolve_sanctioned(fragment, &scanned).err())
        .collect();

    assert!(
        problems.is_empty(),
        "SANCTIONED_PATHS entries must each name exactly one scanned file:\n  - {}",
        problems.join("\n  - ")
    );
}

/// Both refusals are pinned directly, so the rule does not depend on today's
/// tree happening to be unambiguous.
#[test]
fn the_sanctioned_path_resolver_refuses_stale_and_ambiguous_entries() {
    let scanned = vec![
        "crates/ironclaw_architecture_tests/tests/reborn_memory_retired_vocabulary.rs".to_string(),
        "crates/ironclaw_memory/src/lib.rs".to_string(),
    ];
    assert_eq!(
        resolve_sanctioned("reborn_memory_retired_vocabulary.rs", &scanned),
        Ok("crates/ironclaw_architecture_tests/tests/reborn_memory_retired_vocabulary.rs")
    );

    let error = resolve_sanctioned("does_not_exist.rs", &scanned)
        .expect_err("a fragment matching nothing must refuse");
    assert!(
        error.contains("matches no scanned file"),
        "expected a stale-entry refusal, got: {error}"
    );

    let mut shadowed = scanned.clone();
    shadowed.push("crates/ironclaw_memory/src/reborn_memory_retired_vocabulary.rs".to_string());
    let error = resolve_sanctioned("reborn_memory_retired_vocabulary.rs", &shadowed)
        .expect_err("a fragment matching two files must refuse");
    assert!(
        error.contains("matches 2 scanned files"),
        "expected an ambiguity refusal naming the count, got: {error}"
    );
}

#[test]
fn reborn_code_never_references_retired_memory_vocabulary() {
    let (hits, scanned) = scan_workspace(&workspace_root());
    assert!(
        scanned.len() >= MIN_SCANNED_FILES,
        "scanned only {} files (floor {MIN_SCANNED_FILES}) — a partial tree must not \
         read as a clean one",
        scanned.len()
    );
    assert!(
        hits.is_empty(),
        "retired memory vocabulary reintroduced (the bound provider's manifest \
         is the single source of truth: `[[tools]]` is the tool surface, \
         `[memory].lifecycle` gates every host-initiated call):\n{}",
        hits.join("\n")
    );
}

/// The floor's premise, pinned on a fixture: a readable partial tree scans
/// clean (empty hit list) with a file count the floor rejects — exactly the
/// state that must never read as green.
#[test]
fn a_partial_tree_scans_clean_and_hits_the_floor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let family = root.join("crates/domains/ironclaw_memory/src");
    std::fs::create_dir_all(&family).expect("family tree");
    for index in 0..10 {
        std::fs::write(family.join(format!("m{index}.rs")), "pub fn f() {}\n").expect("source");
    }

    let mut hits = Vec::new();
    let mut scanned = Vec::new();
    scan_dir(root, &root.join("crates"), &mut hits, &mut scanned)
        .expect("a readable partial tree scans");
    assert_eq!(
        scanned.len(),
        10,
        "expected the partial tree, got {scanned:?}"
    );
    assert!(
        hits.is_empty(),
        "a partial scan also yields an empty hit list — that is the whole problem"
    );
    assert!(
        scanned.len() < MIN_SCANNED_FILES,
        "the floor must reject this scan"
    );
}
