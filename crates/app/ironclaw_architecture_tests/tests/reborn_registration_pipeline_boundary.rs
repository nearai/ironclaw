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
//!
//! # Discovery is inventory-driven, and the gate asserts it measured something
//!
//! Every scope this gate resolves — the workspace root, the files registration
//! owns, the crate excluded from its own scan — used to be keyed to the literal
//! flat `crates/<name>/` tree shape. The target architecture moves crates into
//! family directories (`crates/<family>/ironclaw_*`,
//! `docs/reborn/target-architecture/PROPOSAL.md` §5), and at the first family
//! `git mv` all three stop resolving at once: `workspace_root()` walked up a
//! fixed two levels and landed on `crates/`, so the scan found no directory,
//! visited **zero files**, and reported success. Green while enforcing nothing
//! (#6963).
//!
//! So identity is anchored on the **crate name** and position on **where that
//! crate's `Cargo.toml` actually is**, at any depth. `CrateInventory` is the
//! same outermost-wins rule that `scripts/ci/lib/crate_tree.py` owns for the
//! Python-side gates — read that module for the rationale; the two are
//! deliberately recognizable as one rule.
//!
//! And discovery working is itself asserted (`measured_scan`): an inventory
//! below the floor, a scan that visited nothing, or an owned scope that
//! resolves to zero real files is an error, never a quiet pass. The last one is
//! load-bearing — WS6 retires one of the two `hosted_mcp_` scopes and the WS7
//! family move retires both, and a scope that has silently stopped matching is
//! how this gate would come back as a false positive against the registration
//! pipeline's own owner.

// Each integration-test binary compiles the shared module independently; this
// binary uses only the workspace-root search.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ratchet_support::{find_workspace_root, workspace_root};

/// Registration-pipeline vocabulary. Naming any of these outside the owning
/// files below means a registration concept has entered generic code.
const TERMS: &[&str] = &["PreparationRequirement", "initial_preparation"];

/// Scopes that legitimately own the registration pipeline and may name its
/// vocabulary freely, as `(crate name, path prefix inside that crate)`. The
/// crate is resolved through the inventory, so these survive a family move; the
/// in-crate half is the part a rename inside the crate must update.
const OWNED_SCOPES: &[(&str, &str)] = &[
    ("ironclaw_extension_host", "src/hosted_mcp_"),
    ("ironclaw_extension_registry", "src/hosted_mcp_"),
];

/// Crates excluded wholesale: this architecture crate names the terms on
/// purpose (in this very file), same footing as the sibling gates.
const EXCLUDED_CRATES: &[&str] = &["ironclaw_architecture_tests"];

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

/// Fail-closed floors. Both are sanity floors, not ratchets: they sit far below
/// the real numbers (66 crate directories and 1,249 scanned files at the WS0
/// baseline) and are never expected to bind. They exist so that "the tree moved
/// out from under this gate" cannot read as "nothing to report".
const MIN_CRATE_DIRECTORIES: usize = 20;
const MIN_SCANNED_FILES: usize = 400;

const PRODUCTION_FLOORS: Floors = Floors {
    crate_directories: MIN_CRATE_DIRECTORIES,
    scanned_files: MIN_SCANNED_FILES,
};

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
    let outcome = match measured_scan(&root, PRODUCTION_FLOORS) {
        Ok(outcome) => outcome,
        Err(error) => panic!(
            "registration-pipeline boundary gate could not measure the tree: {error}\n\n\
             This gate refuses to report success on a scan it cannot vouch for. See \
             docs/reborn/target-architecture/CHECKLIST.md (WS0 blocking prerequisite) and #6963."
        ),
    };

    let allowed: BTreeSet<(String, String)> = ALLOWLIST
        .iter()
        .map(|(path, term)| ((*path).to_string(), (*term).to_string()))
        .collect();

    let new_violations: Vec<&(String, String)> = outcome
        .hits
        .iter()
        .filter(|hit| !allowed.contains(*hit))
        .collect();
    let stale_entries: Vec<&(String, String)> = allowed
        .iter()
        .filter(|entry| !outcome.hits.contains(*entry))
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

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Sanity floors a measured scan must clear before its result means anything.
#[derive(Clone, Copy)]
struct Floors {
    crate_directories: usize,
    scanned_files: usize,
}

/// What a scan visited, not just what it found. The counts are the evidence
/// that the gate measured something.
#[derive(Debug, Default)]
struct ScanOutcome {
    hits: BTreeSet<(String, String)>,
    scanned_files: usize,
    /// Files matched by each resolved owned prefix. A zero here means the scope
    /// no longer names anything on disk.
    owned_prefix_files: BTreeMap<String, usize>,
}

/// Resolved, repo-relative path prefixes for this run's tree shape.
#[derive(Debug)]
struct ResolvedScopes {
    owned: Vec<String>,
    excluded: Vec<String>,
}

/// Every crate directory under `crates/`, repo-relative, outermost-wins.
///
/// Same rule as `scripts/ci/lib/crate_tree.py`: a crate directory is the
/// outermost directory under `crates/` that owns a `Cargo.toml`, so
/// `crates/ironclaw_safety/fuzz/` and the bundled `assets/*/wasm-src/` guests
/// are attributed to the crate that already encloses them.
struct CrateInventory {
    directories: Vec<String>,
}

impl CrateInventory {
    fn discover(root: &Path) -> Result<Self, String> {
        let crates_root = root.join("crates");
        if !crates_root.is_dir() {
            return Err(format!(
                "no crates/ directory under {} — crate discovery cannot run, so the \
                 registration-boundary scan has no scope to resolve",
                root.display()
            ));
        }

        let mut manifests = Vec::new();
        collect_manifest_directories(&crates_root, root, &mut manifests)?;

        // Shallowest first so the outermost owner of a path is always seen
        // before any manifest nested inside it.
        manifests.sort_by_key(|relative| (relative.matches('/').count(), relative.clone()));

        let mut directories: Vec<String> = Vec::new();
        for relative in manifests {
            if directories
                .iter()
                .any(|kept| relative == *kept || relative.starts_with(&format!("{kept}/")))
            {
                continue;
            }
            directories.push(relative);
        }
        directories.sort();
        Ok(Self { directories })
    }

    fn len(&self) -> usize {
        self.directories.len()
    }

    /// The crate directory whose basename is `name`.
    ///
    /// Absent or ambiguous is an error: a crate this gate names cannot quietly
    /// resolve to "nothing" because it moved or was renamed.
    fn directory_named(&self, name: &str) -> Result<&str, String> {
        let matches: Vec<&String> = self
            .directories
            .iter()
            .filter(|directory| directory.rsplit('/').next() == Some(name))
            .collect();
        match matches.as_slice() {
            [single] => Ok(single.as_str()),
            _ => Err(format!(
                "expected exactly one crate directory named {name:?} under crates/, found {}: \
                 {matches:?}. If the crate was renamed or moved, repoint OWNED_SCOPES / \
                 EXCLUDED_CRATES rather than letting this gate scan an unbounded tree.",
                matches.len()
            )),
        }
    }
}

fn collect_manifest_directories(
    dir: &Path,
    root: &Path,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| read_failure(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| entry_failure(dir, error))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Build outputs, vendored frontend packages, and dotted directories
            // can hold manifests that are not workspace crates (`cargo-fuzz`
            // builds into `crates/<x>/fuzz/target/` by default; a pnpm install
            // fills `crates/ironclaw_webui/frontend/node_modules` with thousands
            // of packages, some of which ship a `Cargo.toml` and `src/*.rs`).
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            collect_manifest_directories(&path, root, out)?;
        } else if name == "Cargo.toml"
            && let Some(parent) = path.parent()
            && let Ok(relative) = parent.strip_prefix(root)
        {
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// One phrasing for every I/O refusal in this gate, so the message reads the
/// same whichever walk hit it.
///
/// Dropping these errors was a fail-open: a single unreadable crate `src/` tree
/// keeps `scanned_files` and every owned-scope count above their floors, so the
/// gate reported "no violations" for a subtree it never read (CHECKLIST WS0,
/// #6963).
fn read_failure(path: &Path, error: std::io::Error) -> String {
    format!(
        "failed to read {} — an unreadable path must fail this gate, not vanish from the \
         scan: {error}",
        path.display()
    )
}

fn entry_failure(dir: &Path, error: std::io::Error) -> String {
    format!("failed to read an entry in {}: {error}", dir.display())
}

/// Resolve `OWNED_SCOPES` / `EXCLUDED_CRATES` against this tree's real layout.
fn resolve_scopes(inventory: &CrateInventory) -> Result<ResolvedScopes, String> {
    let mut owned = Vec::new();
    for (crate_name, in_crate_prefix) in OWNED_SCOPES {
        let directory = inventory.directory_named(crate_name)?;
        owned.push(format!("{directory}/{in_crate_prefix}"));
    }
    let mut excluded = Vec::new();
    for crate_name in EXCLUDED_CRATES {
        let directory = inventory.directory_named(crate_name)?;
        excluded.push(format!("{directory}/"));
    }
    Ok(ResolvedScopes { owned, excluded })
}

/// Discover, scan, and refuse to return a result the gate cannot vouch for.
///
/// This is the whole fail-closed contract in one place, so the production call
/// site and the sabotage fixtures drive the same code.
fn measured_scan(root: &Path, floors: Floors) -> Result<ScanOutcome, String> {
    let inventory = CrateInventory::discover(root)?;
    if inventory.len() < floors.crate_directories {
        return Err(format!(
            "crate discovery found only {} crate director(ies) under {}/crates (floor is {}). \
             Either the crate tree moved out from under this gate or the root is wrong",
            inventory.len(),
            root.display(),
            floors.crate_directories
        ));
    }

    let scopes = resolve_scopes(&inventory)?;
    let outcome = scan_tree(root, &scopes)?;

    if outcome.scanned_files < floors.scanned_files {
        return Err(format!(
            "scan visited only {} production file(s) under {}/crates (floor is {}) — a gate \
             that scans nothing must never report success",
            outcome.scanned_files,
            root.display(),
            floors.scanned_files
        ));
    }

    let barren: Vec<&str> = outcome
        .owned_prefix_files
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(prefix, _)| prefix.as_str())
        .collect();
    if !barren.is_empty() {
        return Err(format!(
            "owned scope(s) resolved to zero files on disk: {barren:?}. The registration \
             pipeline moved or was renamed; repoint OWNED_SCOPES in the same PR, or this gate \
             will report the pipeline's own files as violations"
        ));
    }

    Ok(outcome)
}

fn scan_tree(root: &Path, scopes: &ResolvedScopes) -> Result<ScanOutcome, String> {
    let mut outcome = ScanOutcome {
        owned_prefix_files: scopes
            .owned
            .iter()
            .map(|prefix| (prefix.clone(), 0usize))
            .collect(),
        ..ScanOutcome::default()
    };
    collect_hits(&root.join("crates"), root, scopes, &mut outcome)?;
    Ok(outcome)
}

fn is_owned(relative: &str, owned: &[String]) -> bool {
    owned.iter().any(|prefix| relative.starts_with(prefix))
}

fn is_excluded(relative: &str, excluded: &[String]) -> bool {
    excluded.iter().any(|prefix| relative.starts_with(prefix))
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

fn collect_hits(
    dir: &Path,
    root: &Path,
    scopes: &ResolvedScopes,
    outcome: &mut ScanOutcome,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| read_failure(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| entry_failure(dir, error))?;
        let path = entry.path();
        if path.is_dir() {
            // Same exclusions as the manifest walk: build output and vendored
            // frontend packages are not production source, and counting them
            // would inflate `scanned_files` past its floor with files nothing
            // in this workspace wrote.
            let name = entry.file_name();
            if matches!(name.to_string_lossy().as_ref(), "target" | "node_modules") {
                continue;
            }
            collect_hits(&path, root, scopes, outcome)?;
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !is_scannable(&relative) {
            continue;
        }
        for prefix in &scopes.owned {
            if relative.starts_with(prefix) {
                *outcome
                    .owned_prefix_files
                    .entry(prefix.clone())
                    .or_default() += 1;
            }
        }
        if is_owned(&relative, &scopes.owned) || is_excluded(&relative, &scopes.excluded) {
            continue;
        }
        outcome.scanned_files += 1;
        let source = std::fs::read_to_string(&path).map_err(|error| read_failure(&path, error))?;
        for term in scan_source(&source) {
            outcome.hits.insert((relative.clone(), term));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Self-tests — a gate that cannot fail is not a gate
// ---------------------------------------------------------------------------

/// Proves the scanner actually binds.
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

/// Ownership follows the crate inventory, not a hardcoded depth.
///
/// The nested half is the WS7 shape and the coverage this gate was missing:
/// `is_owned` was never exercised at all, so nothing would have noticed the
/// owned scopes silently ceasing to match after a family move.
#[test]
fn ownership_follows_the_crate_inventory_at_any_depth() {
    for family in ["", "substrates/"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        build_synthetic_workspace(root, family);
        let host = format!("crates/{family}ironclaw_extension_host");
        let extensions = format!("crates/{family}ironclaw_extension_registry");
        // Generic code inside the owning crate — the near-miss the prefix must
        // still exclude.
        write_crate(root, &host, &["src/hosted_mcp_manifest.rs", "src/lib.rs"]);

        let inventory = CrateInventory::discover(root).expect("discovery");
        let scopes = resolve_scopes(&inventory).expect("scopes resolve");

        assert!(
            is_owned(&format!("{host}/src/hosted_mcp_manifest.rs"), &scopes.owned),
            "registration's own file must be owned at family depth {family:?}"
        );
        assert!(
            is_owned(
                &format!("{extensions}/src/hosted_mcp_discovery.rs"),
                &scopes.owned
            ),
            "the second owned scope must resolve at family depth {family:?}"
        );
        assert!(
            !is_owned(&format!("{host}/src/lib.rs"), &scopes.owned),
            "the filter must stay a filter: generic code inside the owning crate is not owned"
        );
        assert!(
            !is_owned(
                &format!("crates/{family}ironclaw_assistant/src/lifecycle.rs"),
                &scopes.owned
            ),
            "generic lifecycle code must never be owned"
        );
        assert!(
            is_excluded(
                &format!("crates/{family}ironclaw_architecture_tests/tests/gate.rs"),
                &scopes.excluded
            ),
            "the architecture crate must be excluded at family depth {family:?}"
        );
    }
}

/// The fixed-depth root walk resolved to `crates/` under a family move, which
/// is why the scan went dark. The search form finds the real root instead.
#[test]
fn workspace_root_search_survives_a_family_move() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("workspace manifest");
    let nested = "crates/substrates/ironclaw_architecture_tests";
    write_crate(root, nested, &[]);

    let found = find_workspace_root(&root.join(nested)).expect("root found");
    assert_eq!(
        found.canonicalize().ok(),
        root.canonicalize().ok(),
        "the search must find the workspace root from a family-nested crate"
    );

    // The retired rule: two levels up from the nested crate is `crates/`, and
    // `crates/crates` does not exist — so the old scan found no directory.
    let retired = root
        .join(nested)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    assert_eq!(retired, Some(root.join("crates")));
    assert!(
        !root.join("crates/crates").is_dir(),
        "the retired fixed-depth root made the scan target a directory that cannot exist"
    );
}

/// Discovery below the floor is an error, not an empty answer.
#[test]
fn fail_closed_when_the_crate_inventory_is_missing_or_thin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let error = measured_scan(temp.path(), PRODUCTION_FLOORS).expect_err("no crates/ must fail");
    assert!(
        error.contains("no crates/ directory"),
        "expected a discovery refusal, got: {error}"
    );

    let temp = tempfile::tempdir().expect("tempdir");
    write_crate(temp.path(), "crates/ironclaw_extension_host", &[]);
    let error = measured_scan(temp.path(), PRODUCTION_FLOORS).expect_err("thin tree must fail");
    assert!(
        error.contains("floor is 20"),
        "expected the crate-inventory floor to bind, got: {error}"
    );
}

/// A scan that visited nothing must fail, even with a healthy inventory.
#[test]
fn fail_closed_when_the_scan_visits_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    build_synthetic_workspace(root, "");

    // Everything present and resolvable, but only the owned/excluded files
    // exist — so the generic scan visits zero files.
    let floors = Floors {
        crate_directories: 3,
        scanned_files: 1,
    };
    let error = measured_scan(root, floors).expect_err("an empty scan must fail");
    assert!(
        error.contains("scan visited only 0 production file(s)"),
        "expected the scanned-file floor to bind, got: {error}"
    );

    // One generic file is enough to clear it — the floor binds on emptiness,
    // not on the tree being synthetic.
    write_crate(root, "crates/ironclaw_assistant", &["src/lifecycle.rs"]);
    let outcome = measured_scan(root, floors).expect("a non-empty scan passes");
    assert_eq!(outcome.scanned_files, 1);
    assert!(outcome.hits.is_empty());
}

/// An owned scope that no longer names anything on disk is an error. This is
/// the WS6/WS7 shape: the scope survives as text while meaning nothing.
#[test]
fn fail_closed_when_an_owned_scope_matches_no_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    build_synthetic_workspace(root, "");
    write_crate(root, "crates/ironclaw_assistant", &["src/lifecycle.rs"]);
    // Retire the registration pipeline's files without repointing OWNED_SCOPES.
    std::fs::remove_file(
        root.join("crates/ironclaw_extension_registry/src/hosted_mcp_discovery.rs"),
    )
    .expect("retire the second owned scope");

    let floors = Floors {
        crate_directories: 3,
        scanned_files: 1,
    };
    let error = measured_scan(root, floors).expect_err("a barren owned scope must fail");
    assert!(
        error.contains("owned scope(s) resolved to zero files"),
        "expected the owned-scope assertion to bind, got: {error}"
    );
    assert!(
        error.contains("ironclaw_extension_registry/src/hosted_mcp_"),
        "the error must name the scope that stopped matching, got: {error}"
    );
}

/// An unreadable path fails the scan instead of disappearing from it.
///
/// The floors cannot cover this shape: one unreadable crate `src/` tree leaves
/// `scanned_files` and every owned-scope count comfortably above their floors,
/// so the gate would report "no violations" for a subtree it never read. Both
/// walks and the source read are pinned, each failing for a deterministic,
/// platform-independent reason rather than a chmod that root ignores in a
/// container.
#[test]
fn an_unreadable_path_fails_the_scan_rather_than_disappearing_from_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let not_a_directory = temp.path().join("regular-file");
    std::fs::write(&not_a_directory, "// fixture\n").expect("fixture file");

    let mut manifests = Vec::new();
    let error = collect_manifest_directories(&not_a_directory, temp.path(), &mut manifests)
        .expect_err("the manifest walk must refuse an unreadable directory");
    assert!(
        error.contains("must fail this gate, not vanish from the scan"),
        "expected an I/O refusal naming the path, got: {error}"
    );

    let scopes = ResolvedScopes {
        owned: Vec::new(),
        excluded: Vec::new(),
    };
    let mut outcome = ScanOutcome::default();
    let error = collect_hits(&not_a_directory, temp.path(), &scopes, &mut outcome)
        .expect_err("the hit walk must refuse an unreadable directory");
    assert!(
        error.contains("must fail this gate, not vanish from the scan"),
        "expected an I/O refusal naming the path, got: {error}"
    );

    // A *file* the walk found but cannot read is the harder half: it sits inside
    // an otherwise healthy scan, so no floor can see it. A dangling symlink is
    // the deterministic stand-in — the walk enumerates it as a scannable `.rs`
    // and the read then fails.
    #[cfg(unix)]
    {
        let src = temp.path().join("crates/ironclaw_assistant/src");
        std::fs::create_dir_all(&src).expect("source tree");
        std::os::unix::fs::symlink(src.join("gone"), src.join("lifecycle.rs"))
            .expect("dangling source");
        let mut outcome = ScanOutcome::default();
        let error = collect_hits(&src, temp.path(), &scopes, &mut outcome)
            .expect_err("an unreadable source must refuse");
        assert!(
            error.contains("lifecycle.rs") && error.contains("must fail this gate"),
            "expected the refusal to name the unreadable source, got: {error}"
        );
    }
}

/// A crate this gate names going missing is an error, not a skipped scope.
#[test]
fn fail_closed_when_a_named_crate_cannot_be_resolved() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    build_synthetic_workspace(root, "");
    std::fs::remove_dir_all(root.join("crates/ironclaw_extension_registry"))
        .expect("remove a named crate");

    let inventory = CrateInventory::discover(root).expect("discovery");
    let error = resolve_scopes(&inventory).expect_err("an absent named crate must fail");
    assert!(
        error.contains("ironclaw_extension_registry"),
        "the error must name the unresolvable crate, got: {error}"
    );
}

/// Nested manifests are attributed to the crate that encloses them, so a
/// vendored guest cannot shadow its owner. Same rule as `crate_tree.py`.
#[test]
fn inventory_attributes_nested_manifests_to_the_outermost_owner() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_crate(root, "crates/ironclaw_safety", &["src/lib.rs"]);
    write_crate(root, "crates/ironclaw_safety/fuzz", &["src/main.rs"]);
    write_crate(root, "crates/ironclaw_safety/target/debug/vendored", &[]);

    let inventory = CrateInventory::discover(root).expect("discovery");
    assert_eq!(
        inventory.directories,
        vec!["crates/ironclaw_safety".to_string()],
        "the fuzz manifest is inside a crate that already owns its path, and target/ is skipped"
    );
}

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Write a crate directory (a `Cargo.toml` plus the named in-crate files).
fn write_crate(root: &Path, directory: &str, files: &[&str]) {
    let crate_dir = root.join(directory);
    std::fs::create_dir_all(&crate_dir).expect("create crate dir");
    let name = directory.rsplit('/').next().unwrap_or("fixture");
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\n"),
    )
    .expect("write manifest");
    for file in files {
        let path = crate_dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create file dir");
        }
        std::fs::write(&path, "// fixture\n").expect("write fixture file");
    }
}

/// A workspace holding exactly the crates this gate names, at `family` depth
/// (`""` for flat, `"substrates/"` for the WS7 shape).
fn build_synthetic_workspace(root: &Path, family: &str) {
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("workspace manifest");
    write_crate(
        root,
        &format!("crates/{family}ironclaw_extension_host"),
        &["src/hosted_mcp_manifest.rs"],
    );
    write_crate(
        root,
        &format!("crates/{family}ironclaw_extension_registry"),
        &["src/hosted_mcp_discovery.rs"],
    );
    write_crate(
        root,
        &format!("crates/{family}ironclaw_architecture_tests"),
        &["src/lib.rs"],
    );
}
