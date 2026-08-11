//! No build script may derive the repository root by a fixed number of
//! `.parent()` hops from `CARGO_MANIFEST_DIR`.
//!
//! WS10, "loud path-pattern inventory". A fixed-depth root encodes "this crate
//! sits directly under `crates/`", which the family move
//! (`crates/<family>/ironclaw_*`, PROPOSAL §5) makes false for every crate at
//! once. What makes it worth a gate rather than a fix-and-forget is the
//! **failure direction**: `ironclaw_extension_host/build.rs` resolved the root
//! two hops up and then read `<root>/skills`. One level deeper, that root is
//! `crates/`, `crates/skills` does not exist, and the build script writes `[]`
//! for both embedded-skill bundles and returns `Ok(())`. The build stays green
//! and the shipped binary silently loses every bundled Reborn skill.
//!
//! `ratchet_support::find_workspace_root` states the replacement rule — search
//! upward for the nearest ancestor holding both `crates/` and `Cargo.toml` —
//! and #6996 already deleted the same idiom's twelve copies from this crate.
//! This gate keeps it out of the build scripts too.
//!
//! Scope note: a build script may still walk to its OWN crate directory or
//! below (`manifest_dir.join("frontend")` in `ironclaw_webui/build.rs` is
//! correct and untouched). Only escaping the crate by counted hops is banned.

mod ratchet_support;

use std::path::{Path, PathBuf};

use ratchet_support::{crate_directories, strip_comments_and_strings, workspace_root};

/// Two or more chained parent-ish hops with no search in between: the shape
/// that means "I know exactly how deep I am". Matched on comment-and-string
/// stripped source so prose describing the anti-pattern does not trip it.
const FIXED_DEPTH_ROOT_SHAPES: &[&str] = &[
    ".parent().and_then(Path::parent)",
    ".parent().and_then(|parent|parent.parent())",
    ".parent().unwrap().parent()",
    ".parent()?.parent()",
    "..\\..\\",
];

fn build_scripts(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = crate_directories(root)
        .into_iter()
        .map(|directory| root.join(directory).join("build.rs"))
        .filter(|path| path.is_file())
        .collect();
    let workspace_build = root.join("build.rs");
    if workspace_build.is_file() {
        found.push(workspace_build);
    }
    found.sort();
    found
}

#[test]
fn reborn_build_scripts_do_not_derive_the_repo_root_by_counted_parent_hops() {
    let root = workspace_root();
    let scripts = build_scripts(&root);

    // Non-vacuity: this gate is a source scan, and a scan that finds no files
    // reports success having checked nothing — the exact failure WS10 exists
    // to close. There are two build scripts under `crates/` today.
    assert!(
        scripts.len() >= 2,
        "expected to find the workspace's build scripts, found {scripts:?}. A build-script \
         scan that discovers nothing passes while checking nothing \
         (docs/reborn/target-architecture/CHECKLIST.md WS10)."
    );

    let mut violations = Vec::new();
    for script in &scripts {
        let source = std::fs::read_to_string(script)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", script.display()));
        let stripped = strip_comments_and_strings(&source);
        let condensed: String = stripped.chars().filter(|c| !c.is_whitespace()).collect();
        for shape in FIXED_DEPTH_ROOT_SHAPES {
            let needle: String = shape.chars().filter(|c| !c.is_whitespace()).collect();
            if condensed.contains(&needle) {
                violations.push(format!(
                    "{}: contains `{shape}`",
                    script.strip_prefix(&root).unwrap_or(script).display()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "a build script derives a path by a counted number of parent hops, which encodes the \
         crate's depth under `crates/`. Search upward for the nearest ancestor holding both \
         `crates/` and `Cargo.toml` instead (see `ironclaw_extension_host/build.rs`), and fail \
         loudly when there is none — the family move (PROPOSAL §5) otherwise silently \
         redirects the root into `crates/`:\n{}",
        violations.join("\n")
    );
}

/// The matcher must actually catch the shape it bans — a gate whose pattern
/// never matches is the same dark verdict one level up.
#[test]
fn reborn_fixed_depth_matcher_catches_the_banned_shapes_and_ignores_prose() {
    let condense = |text: &str| -> String {
        strip_comments_and_strings(text)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    let matches = |text: &str| -> bool {
        let condensed = condense(text);
        FIXED_DEPTH_ROOT_SHAPES.iter().any(|shape| {
            let needle: String = shape.chars().filter(|c| !c.is_whitespace()).collect();
            condensed.contains(&needle)
        })
    };

    // Positive: every banned spelling, including the line-wrapped form the
    // real defect used.
    assert!(matches(
        "let root = manifest_dir.parent().and_then(Path::parent).unwrap();"
    ));
    assert!(matches(
        "let repo_root = manifest_dir\n    .parent()\n    .and_then(Path::parent)\n    .ok_or(e)?;"
    ));
    assert!(matches(
        "let root = dir.parent().unwrap().parent().unwrap();"
    ));
    assert!(matches("let root = dir.parent()?.parent()?;"));
    assert!(matches(
        "let root = manifest_dir.parent().and_then(|parent| parent.parent());"
    ));

    // Negative: the sanctioned search, an in-crate join, and a single hop.
    assert!(!matches(
        "while let Some(d) = current { if d.join(\"crates\").is_dir() { return Ok(d); } \
         current = d.parent(); }"
    ));
    assert!(!matches("let frontend = manifest_dir.join(\"frontend\");"));
    assert!(!matches("let sibling = manifest_dir.parent().unwrap();"));

    // Negative: the ban lives in a doc comment and a string literal, which the
    // stripper must blank — otherwise this very file could not describe it.
    assert!(!matches(
        "/// never write .parent().and_then(Path::parent)\nfn ok() {}"
    ));
    assert!(!matches(
        "let message = \".parent().and_then(Path::parent) is banned\";"
    ));
}
