//! The checked-in bundled catalog must pass the routing-metadata lint.
//!
//! Kept as the enforcement point for epic #6941 criterion 7. The scorer changes this lint
//! shipped alongside in #6937 were closed as the wrong change -- the scorer decides nothing
//! once selection is `ExplicitOnly` -- but the lint itself became MORE relevant, not less:
//! under model-decided selection the description is the model's only signal in the listing, so
//! a generic or over-long one degrades routing for every later request.

use ironclaw_skills::parse_skill_md;

/// The repository root, found by ascending from this crate rather than counting `..` segments.
///
/// `../../skills` was correct while this crate sat at `crates/ironclaw_skills`, and silently became
/// `crates/skills` when WS7 moved it to `crates/domains/ironclaw_skills` — the lint then linted zero
/// skills and the test failed on an empty catalog. Ascending to the marker file survives the next
/// move too.
fn repo_root() -> std::path::PathBuf {
    let mut dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("Cargo.lock").is_file() && dir.join("skills").is_dir() {
            return dir.to_path_buf();
        }
        dir = dir
            .parent()
            .expect("the repository root must be an ancestor of this crate");
    }
}

/// Every checked-in skill must pass the routing-metadata lint.
///
/// Epic #6565 names the failures this prevents: `tech-debt-tracker` declaring `hack`, and
/// `coding` declaring keywords so generic it fired on ~220 of 328 real prompts. Neither
/// boundary matching nor a score threshold can fix those -- they are legitimate whole-word hits
/// on terms that do not identify a skill. The only fix is not declaring them, and the only way
/// that stays fixed is a lint in CI.
///
/// This is also where the epic's Hermes comparison lands: Hermes gets its routing accuracy from
/// "metadata discipline and eligibility filtering, not retrieval and reranking".
#[test]
fn the_checked_in_catalog_passes_the_routing_metadata_lint() {
    let skills_root = repo_root().join("skills");
    let mut offenders: Vec<String> = Vec::new();
    let mut linted = 0usize;

    let mut dirs = std::fs::read_dir(&skills_root)
        .expect("bundled skills root must be readable")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();

    for dir in dirs {
        let md = dir.join("SKILL.md");
        if !md.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&md).expect("read SKILL.md");
        let parsed =
            parse_skill_md(&raw).unwrap_or_else(|error| panic!("parse {}: {error}", md.display()));
        linted += 1;
        let problems = ironclaw_skills::lint_skill_routing_metadata(&parsed.manifest);
        if !problems.is_empty() {
            offenders.push(format!(
                "  {}\n{}",
                parsed.manifest.name,
                problems
                    .iter()
                    .map(|problem| format!("     - {problem}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }

    assert!(
        linted >= 15,
        "expected the whole catalog; linted only {linted} skills"
    );
    assert!(
        offenders.is_empty(),
        "{} of {linted} checked-in skills fail the routing-metadata lint:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
