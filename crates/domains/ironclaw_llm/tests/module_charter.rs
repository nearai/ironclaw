//! The sub-owner map in `CLAUDE.md` is a contract, not a comment.
//!
//! PROPOSAL §6.4.13 asks this crate for "internal module charters for its
//! sub-owners". A charter that nobody checks rots within a release: files get
//! added without an owner, and rows survive the files they name. This test
//! pins both directions, so the map can only be wrong on purpose.
//!
//! It deliberately checks *coverage and existence*, not prose. Whether
//! `retry.rs` is really a decorator is a review question; whether every file
//! has exactly one owner is a mechanical one, and that is what is enforced.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `src/`, as a `/`-separated path relative to `src/`.
fn source_files() -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let rel = path
                    .strip_prefix(root)
                    .expect("path under src/")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    let src = crate_root().join("src");
    let mut out = Vec::new();
    walk(&src, &src, &mut out);
    out.sort();
    out
}

/// Parse the `## Sub-owner map` table into `file -> [sub-owner, ...]`.
///
/// A file listed under two sub-owners keeps both entries so the caller can
/// report the ambiguity rather than silently taking the last one.
fn charter_assignments() -> BTreeMap<String, Vec<String>> {
    let doc = std::fs::read_to_string(crate_root().join("CLAUDE.md")).expect("read CLAUDE.md");
    let section = doc
        .split("## Sub-owner map")
        .nth(1)
        .expect("CLAUDE.md must contain a '## Sub-owner map' section");
    // Stop at the next top-level heading so neighbouring tables are not read.
    let section = section.split("\n## ").next().unwrap_or(section);
    parse_sub_owner_table(section)
}

/// The table parser, split out from the file read so a fixture can exercise
/// separator shapes the checked-in `CLAUDE.md` does not currently use.
fn parse_sub_owner_table(section: &str) -> BTreeMap<String, Vec<String>> {
    let mut assignments: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut saw_row = false;
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        // Header (`Sub-owner | ...`) and the separator carry no data. The
        // separator is matched after stripping alignment colons: a table written
        // `|:---|:---|` yields `:---`, which would otherwise parse as a data row
        // and be inserted as an assigned path — `saw_row` then goes true, so the
        // shape guard below stays quiet and the stale assertion reports `:---`
        // instead of a real diagnosis.
        let separator_cell = cells[0].trim_matches(':');
        if cells.len() < 4 || cells[0] == "Sub-owner" || separator_cell.starts_with("---") {
            continue;
        }
        let owner = cells[0].trim_matches('`').to_string();
        for path in cells[3]
            .split(',')
            .map(|entry| entry.trim().trim_matches('`').trim())
            .filter(|entry| !entry.is_empty())
        {
            assignments
                .entry(path.to_string())
                .or_default()
                .push(owner.clone());
        }
        saw_row = true;
    }
    assert!(
        saw_row,
        "the '## Sub-owner map' section parsed to zero table rows — the table \
         shape changed and this gate silently stopped checking anything"
    );
    assignments
}

#[test]
fn every_source_file_has_exactly_one_sub_owner() {
    let files = source_files();
    let assignments = charter_assignments();

    assert!(
        files.len() > 40,
        "expected to walk the whole crate; found only {} files — the walk is \
         broken and this gate would pass vacuously",
        files.len()
    );

    let unassigned: Vec<&String> = files
        .iter()
        .filter(|file| !assignments.contains_key(*file))
        .collect();
    assert!(
        unassigned.is_empty(),
        "{} source file(s) have no sub-owner in CLAUDE.md's '## Sub-owner map'.\n\
         Add each to the row of the concern it belongs to (see PROPOSAL §6.4.13):\n{}",
        unassigned.len(),
        unassigned
            .iter()
            .map(|file| format!("  {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let stale: Vec<&String> = assignments
        .keys()
        .filter(|path| !files.contains(*path))
        .collect();
    assert!(
        stale.is_empty(),
        "{} sub-owner entr(y/ies) name a file that no longer exists — delete \
         them so the map only shrinks:\n{}",
        stale.len(),
        stale
            .iter()
            .map(|path| format!("  {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let duplicated: Vec<String> = assignments
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(path, owners)| format!("  {path} -> {}", owners.join(", ")))
        .collect();
    assert!(
        duplicated.is_empty(),
        "{} file(s) are claimed by more than one sub-owner. A file has exactly \
         one owner; if it genuinely spans two, split it or charge it to the \
         larger half and say so (see `gemini_oauth.rs`):\n{}",
        duplicated.len(),
        duplicated.join("\n")
    );
}

/// An **aligned** separator row (`|:---|`) must not parse as data.
///
/// The regression this pins: the separator was matched with
/// `cells[0].starts_with("---")`, which sees `:---` and lets the row through.
/// `:---` then becomes both a sub-owner and an assigned path, and — worse —
/// `saw_row` goes true, so the zero-rows shape guard stays quiet and the gate
/// reports `:---` as a stale entry instead of diagnosing anything real.
///
/// The checked-in `CLAUDE.md` uses `|---|`, so without this fixture a
/// reversion of the `trim_matches(':')` fix would still pass.
#[test]
fn an_aligned_separator_row_is_not_parsed_as_data() {
    let unaligned = "\n\
        | Sub-owner | Charter | Never | Files |\n\
        |---|---|---|---|\n\
        | `providers` | impls | contracts | `openai.rs` |\n";
    let aligned = "\n\
        | Sub-owner | Charter | Never | Files |\n\
        |:---|:---|:---|:---|\n\
        | `providers` | impls | contracts | `openai.rs` |\n";
    let centred = "\n\
        | Sub-owner | Charter | Never | Files |\n\
        |:---:|:---:|:---:|:---:|\n\
        | `providers` | impls | contracts | `openai.rs` |\n";

    for (label, table) in [
        ("unaligned", unaligned),
        ("left-aligned", aligned),
        ("centred", centred),
    ] {
        let parsed = parse_sub_owner_table(table);
        assert_eq!(
            parsed.keys().collect::<Vec<_>>(),
            vec!["openai.rs"],
            "{label}: only the data row may be parsed; a separator must never \
             contribute a path (got {parsed:?})"
        );
        assert_eq!(
            parsed.get("openai.rs").map(Vec::as_slice),
            Some(["providers".to_string()].as_slice()),
            "{label}: the owner must come from the data row, not the separator"
        );
        assert!(
            !parsed.keys().any(|key| key.contains("---")),
            "{label}: a separator cell leaked in as an assigned path: {parsed:?}"
        );
    }
}
