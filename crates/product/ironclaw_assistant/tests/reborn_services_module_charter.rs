//! The `reborn_services` module-charter map in `CLAUDE.md` is a contract, not a
//! comment.
//!
//! `src/reborn_services.rs` plus its `reborn_services/` submodules is the
//! god-object PROPOSAL §6.9.1 names: *"the `reborn_services` god-object keeps
//! its freeze ratchet and gains a module-charter map (the audited ≥11
//! sub-owners)"*. The freeze ratchet is a different gate and stays —
//! `crates/app/ironclaw_architecture_tests/tests/reborn_service_method_freeze_ratchet.rs`
//! pins the three-word `ProductSurface` vocabulary and that no local service
//! trait comes back. This gate pins the other half of that sentence: which
//! concern a change belongs to *while the file is still one file*, so a later
//! split inherits a decided seam list instead of an argument.
//!
//! It is the sibling of `ironclaw_webui`'s `tests/handlers_module_charter.rs`
//! (WS6, §6.9.4), and deliberately the same shape, down to the failure
//! messages: a map nobody checks rots faster than the code it describes.
//!
//! **Item granularity, not line ranges.** Owners here are conceptual, not
//! positional — `reborn_services.rs` interleaves descriptor constants for a
//! dozen concerns, and a line-range gate would demand exactly the code movement
//! §6.4.15 calls "module-charter work, **not a split**". Every top-level item
//! maps to exactly one owner; positions are irrelevant.
//!
//! It deliberately checks coverage and existence, not prose. Whether
//! `caller_browse_scope` is really `workspace-fs` rather than `projects` is a
//! review question; whether every item has exactly one owner is a mechanical
//! one, and that is what is enforced.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn src_dir() -> PathBuf {
    crate_root().join("src")
}

/// Every top-level item declared in a Rust source file, in declaration order.
///
/// "Top-level" is column zero: items nested inside a function, an `impl`, or a
/// `mod` block are that item's business, not the charter's. `impl` blocks are
/// therefore not walked — `RebornServices`' inherent methods belong to the
/// owner of `RebornServices` itself, which the map assigns to `dispatch`.
fn top_level_items(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev_cfg_test = false;
    for line in source.lines() {
        if line.starts_with(' ') || line.starts_with('\t') || line.is_empty() {
            continue;
        }
        // A `#[cfg(test)] mod …` is test scaffolding, not charter surface.
        let was_cfg_test = prev_cfg_test;
        prev_cfg_test = line.trim() == "#[cfg(test)]";
        if was_cfg_test {
            continue;
        }
        let rest = strip_visibility(line);
        let rest = rest.strip_prefix("unsafe ").unwrap_or(rest);
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let Some((keyword, tail)) = rest.split_once(' ') else {
            continue;
        };
        if !matches!(
            keyword,
            "fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "mod"
        ) {
            continue;
        }
        let name: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            // Modules chart as `mod <name>` so a map row reads unambiguously —
            // a module and a same-named fn/type stay distinct entries.
            if keyword == "mod" {
                out.push(format!("mod {name}"));
            } else {
                out.push(name);
            }
        }
    }
    out
}

/// Drop a leading visibility modifier, whatever its restriction.
///
/// `pub`, `pub(crate)`, `pub(super)`, `pub(self)`, `pub(in path::to::mod)` — a
/// scanner that knows only the first two reads `pub(super)` as the item's
/// *keyword*, which matches nothing in the keyword set, so the item is silently
/// dropped from `charted_surface()`. Silently, because the completeness check
/// compares charter rows against this same list: an item this function cannot
/// see never registers as "unassigned" and never needs a row. That is the
/// fail-open direction for a gate whose whole claim is "every top-level item
/// appears in exactly one row".
fn strip_visibility(line: &str) -> &str {
    let Some(rest) = line.strip_prefix("pub") else {
        return line;
    };
    if let Some(rest) = rest.strip_prefix(' ') {
        return rest;
    }
    if !rest.starts_with('(') {
        // `public_thing` — `pub` was a name prefix, not a visibility.
        return line;
    }
    // `pub(crate)`, `pub(super)`, `pub(self)`, `pub(in a::b)`: skip to the
    // matching `)`. Restriction paths contain no nested parentheses.
    match rest.find(')') {
        Some(close) => rest[close + 1..].strip_prefix(' ').unwrap_or(line),
        None => line,
    }
}

/// Every chartable item, keyed the way `CLAUDE.md` names it: bare for
/// `reborn_services.rs`, `reborn_services/<file>.rs::<name>` for a submodule.
fn charted_surface() -> Vec<String> {
    let dir = src_dir();
    let main =
        std::fs::read_to_string(dir.join("reborn_services.rs")).expect("read reborn_services.rs");
    let mut out = top_level_items(&main);

    let submodules = dir.join("reborn_services");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&submodules)
        .expect("read reborn_services/ submodule directory")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    paths.sort();
    for path in paths {
        let file = path
            .file_name()
            .expect("submodule file name")
            .to_string_lossy()
            .to_string();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for name in top_level_items(&source) {
            out.push(format!("reborn_services/{file}::{name}"));
        }
    }
    out
}

/// Parse the `## \`reborn_services\` module-charter map` table into
/// `item -> [sub-owner, ...]`.
///
/// An item listed under two sub-owners keeps both entries so the caller can
/// report the ambiguity rather than silently taking the last one.
fn charter_assignments() -> BTreeMap<String, Vec<String>> {
    let doc = std::fs::read_to_string(crate_root().join("CLAUDE.md")).expect("read CLAUDE.md");
    let section = doc
        .split("## `reborn_services` module-charter map")
        .nth(1)
        .expect("CLAUDE.md must contain a '## `reborn_services` module-charter map' section");
    // Stop at the next top-level heading so neighbouring tables are not read.
    let section = section.split("\n## ").next().unwrap_or(section);
    parse_charter_table(section)
}

fn parse_charter_table(section: &str) -> BTreeMap<String, Vec<String>> {
    let mut assignments: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut saw_row = false;
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        // Header and separator carry no data. The separator is matched after
        // stripping alignment colons: a table written `|:---|:---|` yields
        // `:---`, which would otherwise parse as a data row, be inserted as an
        // assigned item, and set `saw_row` — leaving the zero-rows shape guard
        // quiet while the gate reports `:---` instead of diagnosing anything.
        let separator_cell = cells[0].trim_matches(':');
        if cells.len() < 4 || cells[0] == "Sub-owner" || separator_cell.starts_with("---") {
            continue;
        }
        let owner = cells[0].trim_matches('`').to_string();
        for item in cells[3]
            .split(',')
            .map(|entry| entry.trim().trim_matches('`').trim())
            .filter(|entry| !entry.is_empty())
        {
            assignments
                .entry(item.to_string())
                .or_default()
                .push(owner.clone());
        }
        saw_row = true;
    }
    assert!(
        saw_row,
        "the '## `reborn_services` module-charter map' section parsed to zero \
         table rows — the table shape changed and this gate silently stopped \
         checking anything"
    );
    assignments
}

/// `top_level_items` must see an item at **every** visibility level.
///
/// Without this fixture the scanner could regress to
/// `strip_prefix("pub(crate) ")`/`strip_prefix("pub ")` and stay green — while
/// the first `pub(super) fn` anyone adds slips out of `charted_surface()`, and
/// therefore never registers as "unassigned" and never needs a charter row.
#[test]
fn top_level_items_sees_every_visibility_level() {
    let source = "\
fn private_fn() {}
pub fn public_fn() {}
pub(crate) fn crate_fn() {}
pub(super) fn super_fn() {}
pub(self) struct SelfStruct;
pub(in crate::reborn_services) enum RestrictedEnum {}
pub(super) async fn super_async_fn() {}
pub(crate) const CRATE_CONST: usize = 1;
    pub(super) fn nested_is_not_top_level() {}
pub_looking_call();
";
    let items = top_level_items(source);
    assert_eq!(
        items,
        vec![
            "private_fn",
            "public_fn",
            "crate_fn",
            "super_fn",
            "SelfStruct",
            "RestrictedEnum",
            "super_async_fn",
            "CRATE_CONST",
        ],
        "every top-level item must be seen regardless of visibility, and indented \
         items must stay excluded"
    );
}

#[test]
fn every_reborn_services_item_has_exactly_one_sub_owner() {
    let items = charted_surface();
    let assignments = charter_assignments();

    assert!(
        items.len() > 400,
        "expected to walk the whole reborn_services surface; found only {} \
         top-level items — the walk is broken and this gate would pass vacuously",
        items.len()
    );
    assert!(
        items
            .iter()
            .any(|item| item.starts_with("reborn_services/")),
        "no submodule item was collected — the `reborn_services/` walk is broken \
         and every submodule row would go unchecked"
    );

    let unassigned: Vec<&String> = items
        .iter()
        .filter(|item| !assignments.contains_key(*item))
        .collect();
    assert!(
        unassigned.is_empty(),
        "{} reborn_services item(s) have no sub-owner in CLAUDE.md's \
         '## `reborn_services` module-charter map'.\nAdd each to the row of the \
         concern it belongs to (see PROPOSAL §6.9.1). A helper belongs to the \
         concern whose request shape it reads, not to `dispatch`, unless more \
         than one concern calls it:\n{}",
        unassigned.len(),
        unassigned
            .iter()
            .map(|item| format!("  {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let stale: Vec<&String> = assignments
        .keys()
        .filter(|item| !items.contains(*item))
        .collect();
    assert!(
        stale.is_empty(),
        "{} charter entr(y/ies) name an item that no longer exists — delete \
         them so the map only shrinks:\n{}",
        stale.len(),
        stale
            .iter()
            .map(|item| format!("  {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let duplicated: Vec<String> = assignments
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(item, owners)| format!("  {item} -> {}", owners.join(", ")))
        .collect();
    assert!(
        duplicated.is_empty(),
        "{} item(s) are claimed by more than one sub-owner. An item has exactly \
         one owner; if it genuinely serves two concerns it belongs to \
         `dispatch`:\n{}",
        duplicated.len(),
        duplicated.join("\n")
    );
}

/// §6.9.1 asks for "the audited **≥11** sub-owners". Pin the floor so a future
/// collapse into three vague buckets fails rather than passing coverage.
#[test]
fn the_map_keeps_at_least_the_audited_sub_owner_count() {
    let owners: std::collections::BTreeSet<String> =
        charter_assignments().into_values().flatten().collect();
    assert!(
        owners.len() >= 11,
        "the map has collapsed to {} sub-owner(s); PROPOSAL §6.9.1 asks for at \
         least 11. Merging concerns here does not make the file smaller, it only \
         makes the eventual split re-litigate the seams: {:?}",
        owners.len(),
        owners
    );
}

/// The `large_file` waiver **stays**. A charter map is explicitly not the split.
///
/// Without this, the natural next move for someone reading the charter is to
/// delete the waiver as "handled" — which would silently drop the file out of
/// the ARCH-SPRAWL tracking `scripts/pre-commit-safety.sh` enforces.
#[test]
fn the_large_file_waiver_survives_the_charter_map() {
    let source = std::fs::read_to_string(src_dir().join("reborn_services.rs"))
        .expect("read reborn_services.rs");
    assert!(
        source.contains("// arch-exempt: large_file"),
        "the `// arch-exempt: large_file` waiver was removed from \
         reborn_services.rs. The module-charter map does not discharge it — \
         PROPOSAL §6.4.15 calls this shape \"module-charter work, not a split\". \
         Restore it, or land the split and delete both."
    );
    assert!(
        source.contains("plan #5985"),
        "the `large_file` waiver no longer names plan #5985. \
         `scripts/pre-commit-safety.sh` requires \
         `// arch-exempt: <category>, <reason>, plan #NNNN`, and the plan number \
         is the only thing that makes the waiver revocable."
    );
}

/// An **aligned** separator row (`|:---|`) must not parse as data.
///
/// The regression this pins: matching the separator with
/// `cells[0].starts_with("---")` sees `:---` and lets the row through. `:---`
/// then becomes both a sub-owner and an assigned item, and `saw_row` goes true,
/// so the zero-rows shape guard stays quiet.
#[test]
fn an_aligned_separator_row_is_not_parsed_as_data() {
    for (label, separator) in [
        ("unaligned", "|---|---|---|---|"),
        ("left-aligned", "|:---|:---|:---|:---|"),
        ("centred", "|:---:|:---:|:---:|:---:|"),
    ] {
        let table = format!(
            "\n| Sub-owner | Owns | Never contains | Items |\n{separator}\n\
             | `runs` | run control | reads | `map_turn_error` |\n"
        );
        let parsed = parse_charter_table(&table);
        assert_eq!(
            parsed.keys().collect::<Vec<_>>(),
            vec!["map_turn_error"],
            "{label}: only the data row may be parsed; a separator must never \
             contribute an item (got {parsed:?})"
        );
        assert_eq!(
            parsed.get("map_turn_error").map(Vec::as_slice),
            Some(["runs".to_string()].as_slice()),
            "{label}: the owner must come from the data row, not the separator"
        );
    }
}
