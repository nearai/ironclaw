//! The sub-owner map in `CLAUDE.md` is a contract, not a comment — and the
//! two-engine severance it describes is an invariant, not an observation.
//!
//! PROPOSAL §6.4.8 asks this crate for the `engine`-vs-`product_auth` split to
//! become "two chartered top-level modules". Both modules already existed; what
//! was missing was the charter, and a charter nobody checks rots within a
//! release. This file pins the two halves that can rot:
//!
//! 1. **Coverage** — every `src/**/*.rs` has exactly one owner and every path
//!    named by an owner exists, so a new file fails until it is charted and a
//!    deleted one fails until its entry goes.
//! 2. **Severance** — `engine` must not name `product_auth` and `product_auth`
//!    must not name `engine`. That is the property the "two engines" framing
//!    *is*; without it the map would describe a split that had quietly closed.
//!
//! Coverage deliberately checks existence, not prose: whether `credential.rs`
//! is really shared vocabulary is a review question; whether every file has
//! exactly one owner is a mechanical one, and that is what is enforced.

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
        // and be inserted as an assigned path.
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
        files.len() > 35,
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
         Add each to the row of the concern it belongs to (see PROPOSAL §6.4.8).\n\
         A file that is neither engine's belongs to `vocabulary` only if BOTH \
         engines name it; if only one does, it belongs to that engine:\n{}",
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
         larger half and say so (see `credential.rs`):\n{}",
        duplicated.len(),
        duplicated.join("\n")
    );
}

/// Concatenate every `.rs` file under `src/<module>/`, minus `//`/`//!` comment
/// lines.
///
/// Comments are stripped because both charters *name the other engine in
/// prose* — deliberately, since the severance is the thing they document — and
/// a scan that counted those would be unsatisfiable by construction.
fn module_code(module: &str) -> String {
    fn walk(dir: &Path, out: &mut String) {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()))
            .map(|entry| entry.expect("dir entry").path())
            .collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
                for line in text.lines() {
                    if !line.trim_start().starts_with("//") {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
        }
    }
    let dir = crate_root().join("src").join(module);
    assert!(
        dir.is_dir(),
        "expected a top-level `src/{module}/` module; the two-engine split \
         described in CLAUDE.md and PROPOSAL §6.4.8 no longer matches the tree"
    );
    let mut out = String::new();
    walk(&dir, &mut out);
    assert!(
        out.len() > 10_000,
        "src/{module}/ concatenated to only {} bytes of code — the walk is \
         broken and this gate would pass vacuously",
        out.len()
    );
    out
}

/// The two engines meet only through the crate root's shared vocabulary.
///
/// This is the property PROPOSAL §6.4.8's "two-engine split" *is*. It held at
/// zero references in both directions when the charter was written; without
/// this test, the first `use crate::engine::…` inside `product_auth` would
/// close the split silently and leave the charter describing something that no
/// longer existed.
#[test]
fn the_two_engines_do_not_name_each_other() {
    let engine = module_code("engine");
    let product_auth = module_code("product_auth");

    for probe in [
        "crate::product_auth",
        "super::product_auth",
        "product_auth::",
    ] {
        assert!(
            !engine.contains(probe),
            "src/engine/ names `{probe}` — the vendor-handshake engine must not \
             reach into the durable product-auth engine. Route the value \
             through the shared vocabulary re-exported from the crate root, or \
             invert the call. (PROPOSAL §6.4.8; charter in engine/mod.rs.)"
        );
    }

    for probe in ["crate::engine", "super::engine", "engine::"] {
        assert!(
            !product_auth.contains(probe),
            "src/product_auth/ names `{probe}` — the durable product-auth \
             engine must not reach into the vendor-handshake engine. A vendor \
             call belongs behind the `AuthProviderClient` port, which is shared \
             vocabulary. (PROPOSAL §6.4.8; charter in product_auth/mod.rs.)"
        );
    }
}

/// An **aligned** separator row (`|:---|`) must not parse as data.
///
/// The regression this pins: matching the separator with
/// `cells[0].starts_with("---")` sees `:---` and lets the row through. `:---`
/// then becomes both a sub-owner and an assigned path, and — worse — `saw_row`
/// goes true, so the zero-rows shape guard stays quiet and the gate reports
/// `:---` as a stale entry instead of diagnosing anything real.
///
/// The checked-in `CLAUDE.md` uses `|---|`, so without this fixture a reversion
/// of the `trim_matches(':')` guard would still pass.
#[test]
fn an_aligned_separator_row_is_not_parsed_as_data() {
    let unaligned = "\n\
        | Sub-owner | Owns | Never contains | Files |\n\
        |---|---|---|---|\n\
        | `engine` | vendor handshake | lifecycle | `engine/dcr.rs` |\n";
    let aligned = "\n\
        | Sub-owner | Owns | Never contains | Files |\n\
        |:---|:---|:---|:---|\n\
        | `engine` | vendor handshake | lifecycle | `engine/dcr.rs` |\n";
    let centred = "\n\
        | Sub-owner | Owns | Never contains | Files |\n\
        |:---:|:---:|:---:|:---:|\n\
        | `engine` | vendor handshake | lifecycle | `engine/dcr.rs` |\n";

    for (label, table) in [
        ("unaligned", unaligned),
        ("left-aligned", aligned),
        ("centred", centred),
    ] {
        let parsed = parse_sub_owner_table(table);
        assert_eq!(
            parsed.keys().collect::<Vec<_>>(),
            vec!["engine/dcr.rs"],
            "{label}: only the data row may be parsed; a separator must never \
             contribute a path (got {parsed:?})"
        );
        assert_eq!(
            parsed.get("engine/dcr.rs").map(Vec::as_slice),
            Some(["engine".to_string()].as_slice()),
            "{label}: the owner must come from the data row, not the separator"
        );
        assert!(
            !parsed.keys().any(|key| key.contains("---")),
            "{label}: a separator cell leaked in as an assigned path: {parsed:?}"
        );
    }
}
