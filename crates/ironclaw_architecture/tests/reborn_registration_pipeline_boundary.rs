//! Registration-pipeline boundary gate.
//!
//! Hosted-MCP **registration** is its own pipeline: it validates a
//! user-supplied endpoint, authenticates to it, discovers its tools, and only
//! then hands the shared extension lifecycle a *complete* package —
//! indistinguishable from a bundled one. Everything about "registered but not
//! yet discovered" belongs inside that pipeline.
//!
//! The shared lifecycle (install → configure → activate → execute → remove)
//! runs for every extension: gmail, slack, github, telegram. It must not learn
//! registration's vocabulary. When it does, a gate built for one hosted-MCP
//! package silently changes the resting state of packages that have nothing to
//! do with MCP — the concrete failure that motivated this gate was a
//! first-party extension pinned at `setup_needed` forever by a preparation
//! check it should never have reached.
//!
//! This is the same shape as `reborn_extension_specificity.rs` and
//! `reborn_retired_taxonomy.rs`: a scanner plus a shrink-only allowlist.
//!
//! Allowlist discipline: `ALLOWLIST` enumerates today's leaked `(path, term)`
//! pairs. A new pair fails. A *stale* pair — the file no longer names the term
//! — also fails, so removing a leak forces the entry out and the list can only
//! shrink. `REGISTRATION_BOUNDARY_ALLOWLIST_BASELINE` is the other half: the
//! list cannot grow untracked either. Lower it in the same PR that deletes
//! entries so the new floor is locked in. The target is an empty list.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Registration-pipeline vocabulary. Naming any of these outside the owning
/// files below means a registration concept has entered generic code.
const TERMS: &[&str] = &["PreparationRequirement", "initial_preparation"];

/// Files that legitimately own the registration pipeline and may name its
/// vocabulary freely. A path is owned when it *starts with* one of these.
const OWNED_PREFIXES: &[&str] = &[
    "crates/ironclaw_extension_host/src/hosted_mcp_",
    "crates/ironclaw_extensions/src/hosted_mcp_",
];

/// Crates excluded wholesale: this architecture crate names the terms on
/// purpose (in this very file), same footing as the sibling gates.
const EXCLUDED_CRATES: &[&str] = &["ironclaw_architecture"];

/// Leaked `(relative path, term)` pairs. **Empty, and it stays empty.**
///
/// The shared lifecycle no longer reasons about registration state at all:
/// "capabilities not resolved yet" is derived from the package via
/// `ResolvedExtensionManifest::has_model_visible_capabilities`, so nothing
/// generic carries or reads a stored readiness flag. Do NOT add an entry to
/// make a new change compile — put the concept inside the registration
/// pipeline instead.
const ALLOWLIST: &[(&str, &str)] = &[];

/// Ceiling on `ALLOWLIST`, so the list cannot grow untracked. Lower it in the
/// same PR that removes entries. Now at the target: 0.
const REGISTRATION_BOUNDARY_ALLOWLIST_BASELINE: usize = 0;

#[test]
fn registration_boundary_allowlist_ratchets_down_only() {
    // Bound through a binding: the baseline is 0 today, and comparing a
    // `usize` against the literal minimum folds to a constant that clippy
    // rejects. The comparison stays written this way so the ratchet still
    // reads correctly if the baseline is ever deliberately raised.
    let baseline = REGISTRATION_BOUNDARY_ALLOWLIST_BASELINE;
    assert!(
        ALLOWLIST.len() <= baseline,
        "registration-boundary ALLOWLIST grew to {} entries (baseline {}): this list is \
         shrink-only. Registration owns endpoint validation, MCP-server auth, discovery, retry, \
         and any not-yet-discovered state; the shared lifecycle receives only a complete \
         package. Move the concept into the registration pipeline rather than allowlisting a \
         new pair. If the owner has approved a deliberate carve-out, raise \
         REGISTRATION_BOUNDARY_ALLOWLIST_BASELINE in the same PR with the rationale in the PR \
         body.",
        ALLOWLIST.len(),
        REGISTRATION_BOUNDARY_ALLOWLIST_BASELINE
    );
}

#[test]
fn registration_concepts_stay_inside_the_registration_pipeline() {
    let root = workspace_root();
    let mut hits: BTreeSet<(String, String)> = BTreeSet::new();
    collect_hits(&root.join("crates"), &root, &mut hits);

    let allowed: BTreeSet<(String, String)> = ALLOWLIST
        .iter()
        .map(|(path, term)| ((*path).to_string(), (*term).to_string()))
        .collect();

    let new_violations: Vec<&(String, String)> =
        hits.iter().filter(|hit| !allowed.contains(*hit)).collect();
    let stale_entries: Vec<&(String, String)> = allowed
        .iter()
        .filter(|entry| !hits.contains(*entry))
        .collect();

    let mut failures = Vec::new();
    if !new_violations.is_empty() {
        failures.push(format!(
            "registration-pipeline vocabulary in generic lifecycle code (move the concept into \
             the registration pipeline — the shared lifecycle must receive a complete package \
             and learn nothing about discovery):\n{}",
            new_violations
                .iter()
                .map(|(path, term)| format!("    (\"{path}\", \"{term}\"),"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !stale_entries.is_empty() {
        failures.push(format!(
            "stale ALLOWLIST entries (the file no longer names the term — delete the entries \
             and lower REGISTRATION_BOUNDARY_ALLOWLIST_BASELINE; the allowlist only \
             shrinks):\n{}",
            stale_entries
                .iter()
                .map(|(path, term)| format!("    (\"{path}\", \"{term}\"),"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    assert!(
        failures.is_empty(),
        "registration-pipeline boundary gate failed:\n\n{}",
        failures.join("\n\n")
    );
}

/// Proves the scanner actually binds: a gate that cannot fail is not a gate.
/// Mirrors the self-test discipline the sibling specificity gate uses.
#[test]
fn scanner_flags_a_planted_violation() {
    let generic =
        "fn phase(p: PreparationRequirement) -> bool { p == PreparationRequirement::Ready }";
    assert!(
        !scan_source(generic).is_empty(),
        "scanner missed a registration term in generic source — the gate would pass while \
         enforcing nothing"
    );

    let stripped = "#[cfg(test)]\nmod tests {\n    use super::PreparationRequirement;\n}\n";
    assert!(
        scan_source(stripped).is_empty(),
        "scanner matched inside a #[cfg(test)] block — test code may name the vocabulary"
    );
}

/// Terms named by `source`, with `#[cfg(test)]` blocks stripped first.
fn scan_source(source: &str) -> BTreeSet<String> {
    let body = strip_cfg_test(source);
    TERMS
        .iter()
        .filter(|term| body.contains(**term))
        .map(|term| (*term).to_string())
        .collect()
}

/// Remove `#[cfg(test)]`-attributed items by brace matching. Test code may
/// name the registration vocabulary (overview §8, same as the sibling gates).
fn strip_cfg_test(source: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(index) = rest.find(MARKER) {
        out.push_str(&rest[..index]);
        let after = &rest[index + MARKER.len()..];
        let Some(open) = after.find('{') else {
            // Attribute with no block body (e.g. `#[cfg(test)] use ...;`).
            // Drop to the end of that item instead.
            match after.find(';') {
                Some(semi) => {
                    rest = &after[semi + 1..];
                    continue;
                }
                None => return out,
            }
        };
        let mut depth = 0usize;
        let mut end = None;
        for (offset, ch) in after[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + offset + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(end) => rest = &after[end..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn is_owned(relative: &str) -> bool {
    OWNED_PREFIXES
        .iter()
        .any(|prefix| relative.starts_with(prefix))
}

fn is_excluded(relative: &str) -> bool {
    EXCLUDED_CRATES
        .iter()
        .any(|krate| relative.starts_with(&format!("crates/{krate}/")))
}

/// Scannable: `src/` Rust files only, skipping test-named files. Whole `tests/`
/// directories never enter because the walk only descends into `src/`.
fn is_scannable(relative: &str) -> bool {
    if !relative.ends_with(".rs") {
        return false;
    }
    if !relative.contains("/src/") {
        return false;
    }
    let file = relative.rsplit('/').next().unwrap_or_default();
    if file == "tests.rs" || file.ends_with("_tests.rs") {
        return false;
    }
    !relative.contains("/tests/")
}

fn collect_hits(dir: &Path, root: &Path, hits: &mut BTreeSet<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_hits(&path, root, hits);
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !is_scannable(&relative) || is_owned(&relative) || is_excluded(&relative) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for term in scan_source(&source) {
            hits.insert((relative.clone(), term));
        }
    }
}

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/ironclaw_architecture`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("architecture crate sits two levels below the workspace root")
        .to_path_buf()
}
