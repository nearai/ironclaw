//! §11.2.7 cross-crate `include_str!` / `include_bytes!` scan — **WARN MODE**.
//!
//! The rule (PROPOSAL §11.2 item 7): a relative include path that escapes its
//! own crate root is a compile-time reach into someone else's tree. It bypasses
//! the dependency graph the boundary tests police — `extension_host` has no
//! cargo edge to `ironclaw_first_party_extensions`, yet it compiles that
//! crate's manifests into itself — and it silently breaks the moment either
//! crate moves, which is precisely what CHECKLIST WS7's family `git mv` does to
//! every path in the tree.
//!
//! **This scan does not fail on the inventory it finds.** Arming it as a gate
//! today would fail CI on ~60 pre-existing sites that only WS1–WS4 can fix
//! (`providers.json` becoming a crate asset, `wit/` moving into the wasm lane,
//! package colocation). Warn mode makes that debt visible and countable now,
//! at WS0, so the later PRs can prove they shrank it.
//!
//! **Who flips it to enforcing:** the WS2 PR that lands CHECKLIST WS2's row
//! *"Kill the cross-crate `include_str!` reach-ins (gmail/github/nearai-mcp
//! manifests): catalog/manifest data flows from package inventory via the
//! binary; verify with the new §11.2.7 scan"* (workstream #6920, epic #3773).
//! That PR sets `REPORT_ONLY = false` below, at which point the `cross-crate`
//! findings become assertions. The `repo-root asset` half stays reported until
//! its owners land (WS1 `providers.json`, WS4 `wit/`), then the whole scan is
//! enforcing and this comment goes away with `REPORT_ONLY`.
//!
//! To read the inventory:
//!
//! ```text
//! cargo test -p ironclaw_architecture --test reborn_cross_crate_include_scan -- --nocapture
//! ```
//!
//! Scope: every **workspace member** package (resolved through `cargo
//! metadata`, so family moves in WS7 cannot silently narrow it), walking its
//! whole tree except `target/` and any nested package directory — a nested
//! package is scanned under its own root, or, if it is workspace-excluded
//! (the `assets/*/wasm-src` guests), not at all. The scan reads raw source:
//! it cannot strip comments the way the sibling ratchets do, because the
//! literal it needs to read *is* the string the stripper would blank. A
//! commented-out include is therefore a (harmless, warn-mode) false positive;
//! this file skips itself for that reason, since its own fixtures are made of
//! include-shaped strings.
//!
//! Not to be confused with `scripts/ci/check-include-str-paths.sh`, which is
//! already enforcing and asks a different question: does every `include_str!`
//! target *exist* and is it inside each Dockerfile's build context (the #5603
//! outage class)? A path can satisfy that and still be a boundary violation
//! here — `providers.json` exists and is COPYd, and is still a reach outside
//! `ironclaw_llm`. The two are complementary; neither subsumes the other.

// Only `workspace_root` is reachable from this binary; the type-def scanners in
// the shared module belong to the sibling ratchet binaries.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use ratchet_support::workspace_root;
use serde_json::Value;

/// Flipped to `false` by the WS2 PR named in the module docs. While it is
/// `true` the scan reports and never fails on findings; the self-tests below
/// keep the scanner itself honest in the meantime.
const REPORT_ONLY: bool = true;

/// WS0 inventory size, measured on this checkout (`origin/main` @ `ae0989c37`,
/// 2026-07-30) by running this test: 62 escaping include sites — 19 reaching
/// into another workspace crate, 43 reaching repo-root assets. Reported
/// against today's count so the delta is visible without re-deriving it.
const WS0_ESCAPING_INCLUDE_SITES: usize = 62;
const WS0_CROSS_CRATE_INCLUDE_SITES: usize = 19;

/// This file's own fixtures are include-shaped strings; skip it the way the
/// shared type-def scanner skips the ratchet files (defense in depth).
const SKIP_FILES: &[&str] = &["reborn_cross_crate_include_scan.rs"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct EscapingInclude {
    /// Workspace package the offending file belongs to.
    crate_name: String,
    /// Workspace-relative path of the file holding the macro.
    file: String,
    line: usize,
    /// The literal written in the source.
    literal: String,
    /// Workspace-relative path the literal resolves to.
    resolved: String,
    /// The workspace package that owns the target, when one does — the
    /// cross-crate reach-ins §11.2.7 exists for. `None` means a repo-root
    /// asset (`providers.json`, `wit/`, `migrations/`, `tests/fixtures/`).
    target_crate: Option<String>,
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

/// Every `include_str!`/`include_bytes!` literal in `source`, with the 1-based
/// line it starts on. Handles the rustfmt-split form (the macro name, its
/// paren, and the literal on separate lines) and the `concat!("<prefix>", …)`
/// form used for per-tool schema/prompt tables, where the leading literal is
/// the part that determines which tree is being reached into.
fn include_literals(source: &str) -> Vec<(usize, String)> {
    const MACROS: [&str; 2] = ["include_str!", "include_bytes!"];
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut cursor = 0usize;
    while cursor < source.len() {
        let Some((offset, macro_name)) = MACROS
            .iter()
            .filter_map(|name| source[cursor..].find(name).map(|at| (cursor + at, *name)))
            .min_by_key(|(at, _)| *at)
        else {
            break;
        };
        let mut index = offset + macro_name.len();
        cursor = index;
        // `(` then optionally `concat!` + `(`, in either case skipping
        // whitespace — then the literal must be next, or this is a form the
        // scan does not understand (e.g. `concat!(env!(..), ..)`) and is
        // skipped rather than guessed at.
        let expect = |token: &str, index: &mut usize| -> bool {
            while bytes.get(*index).is_some_and(u8::is_ascii_whitespace) {
                *index += 1;
            }
            if source[*index..].starts_with(token) {
                *index += token.len();
                true
            } else {
                false
            }
        };
        if !expect("(", &mut index) {
            continue;
        }
        let mut after_concat = index;
        if expect("concat!", &mut after_concat) && expect("(", &mut after_concat) {
            index = after_concat;
        }
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'"') {
            continue;
        }
        index += 1;
        let literal_start = index;
        while let Some(&byte) = bytes.get(index) {
            match byte {
                // Escapes are stepped over, not decoded: no include path in
                // this tree uses one, and a literal that survives verbatim
                // simply fails to resolve rather than reporting a truncation.
                b'\\' => index += 2,
                b'"' => break,
                _ => index += 1,
            }
        }
        // `get` rather than indexing: a stepped-over escape can land mid-char
        // in non-ASCII source, and a warn-mode scanner must not panic on it.
        let literal = source
            .get(literal_start..index)
            .unwrap_or_default()
            .to_string();
        let line = source[..offset].matches('\n').count() + 1;
        found.push((line, literal));
        cursor = index.max(cursor + 1);
    }
    found
}

/// Lexically resolve `literal` against `from` (the directory holding the
/// source file). Lexical, not `canonicalize`, so a path whose target is a
/// build artifact that has not been produced yet still classifies correctly.
fn resolve(from: &Path, literal: &str) -> PathBuf {
    let mut resolved = from.to_path_buf();
    for component in Path::new(literal).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

fn contains(root: &Path, path: &Path) -> bool {
    path == root || path.starts_with(root)
}

// ---------------------------------------------------------------------------
// Workspace walk
// ---------------------------------------------------------------------------

fn cargo_metadata(root: &Path) -> Value {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON")
}

/// `(package name, package root directory)` for every workspace member.
fn workspace_packages(root: &Path) -> BTreeMap<String, PathBuf> {
    let metadata = cargo_metadata(root);
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let mut roots = BTreeMap::new();
    for package in packages {
        let name = package["name"]
            .as_str()
            .expect("every package must have a name");
        let manifest = Path::new(
            package["manifest_path"]
                .as_str()
                .expect("every package must have a manifest_path"),
        );
        let package_root = manifest
            .parent()
            .expect("a manifest always has a parent directory")
            .to_path_buf();
        roots.insert(name.to_string(), package_root);
    }
    assert!(
        !roots.is_empty(),
        "cargo metadata reported no workspace packages — the scan would be vacuous"
    );
    roots
}

/// The workspace package owning `path`, ignoring the workspace-root package
/// (whose directory contains every other crate and so would match everything).
fn owning_package<'a>(
    packages: &'a BTreeMap<String, PathBuf>,
    workspace_root: &Path,
    path: &Path,
) -> Option<&'a str> {
    packages
        .iter()
        .filter(|(_, package_root)| package_root.as_path() != workspace_root)
        .filter(|(_, package_root)| contains(package_root, path))
        // Deepest match wins, so a package nested under another is attributed
        // to the nearer owner.
        .max_by_key(|(_, package_root)| package_root.components().count())
        .map(|(name, _)| name.as_str())
}

fn walk_rust_files(dir: &Path, package_root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => panic!("failed to read {}: {error}", dir.display()),
    };
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|name| name.to_str());
            if matches!(name, Some("target" | ".git" | "node_modules")) {
                continue;
            }
            // A nested package is scanned under its own root (or, when it is
            // workspace-excluded, not at all) — never as part of its parent.
            if path.join("Cargo.toml").exists() && path != package_root {
                continue;
            }
            walk_rust_files(&path, package_root, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| SKIP_FILES.contains(&name))
        {
            continue;
        }
        out.push(path);
    }
}

fn scan_workspace() -> Vec<EscapingInclude> {
    let root = workspace_root();
    let packages = workspace_packages(&root);
    let relative = |path: &Path| {
        path.strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    };

    let mut findings = Vec::new();
    for (crate_name, package_root) in &packages {
        let mut files = Vec::new();
        walk_rust_files(package_root, package_root, &mut files);
        files.sort();
        for file in files {
            let source = std::fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", file.display()));
            let directory = file
                .parent()
                .expect("a source file always has a parent directory");
            for (line, literal) in include_literals(&source) {
                let resolved = resolve(directory, &literal);
                if contains(package_root, &resolved) {
                    continue;
                }
                findings.push(EscapingInclude {
                    crate_name: crate_name.clone(),
                    file: relative(&file),
                    line,
                    literal,
                    resolved: relative(&resolved),
                    target_crate: owning_package(&packages, &root, &resolved)
                        .map(ToString::to_string),
                });
            }
        }
    }
    findings.sort_by(|left, right| {
        (&left.file, left.line, &left.literal).cmp(&(&right.file, right.line, &right.literal))
    });
    findings
}

fn render(findings: &[EscapingInclude]) -> String {
    findings
        .iter()
        .map(|finding| {
            let target = match &finding.target_crate {
                Some(target) => format!("-> crate {target}"),
                None => "-> repo-root asset".to_string(),
            };
            format!(
                "    {}:{} include \"{}\" {} ({})",
                finding.file, finding.line, finding.literal, target, finding.resolved
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

#[test]
fn reborn_cross_crate_include_paths_are_inventoried() {
    let findings = scan_workspace();
    let (cross_crate, repo_root): (Vec<_>, Vec<_>) = findings
        .iter()
        .cloned()
        .partition(|finding| finding.target_crate.is_some());

    eprintln!(
        "§11.2.7 cross-crate include scan — WARN MODE (WS0 baseline {WS0_ESCAPING_INCLUDE_SITES} \
         sites, {WS0_CROSS_CRATE_INCLUDE_SITES} of them cross-crate)"
    );
    eprintln!(
        "  today: {} escaping include sites — {} into another workspace crate, {} into repo-root assets",
        findings.len(),
        cross_crate.len(),
        repo_root.len()
    );
    eprintln!(
        "\n  cross-crate reach-ins (WS2 turns these into failures):\n{}",
        render(&cross_crate)
    );
    eprintln!(
        "\n  repo-root asset reach-ins (WS1 `providers.json`, WS4 `wit/`, and the migration/\
         fixture owners):\n{}",
        render(&repo_root)
    );

    // Warn mode short-circuits the assertion; flipping REPORT_ONLY makes the
    // cross-crate half of the inventory a hard requirement.
    assert!(
        REPORT_ONLY || cross_crate.is_empty(),
        "§11.2.7 is enforcing now: {} cross-crate include reach-ins must be zero. Route the \
         data through the owning crate's public API or the binary's package inventory instead \
         of compiling another crate's tree into this one:\n{}",
        cross_crate.len(),
        render(&cross_crate)
    );
}

// ---------------------------------------------------------------------------
// Scanner self-tests — the guardrail must fail loudly on its own regressions
// (CHECKLIST WS10: every new scanner lands with positive + negative fixtures).
// ---------------------------------------------------------------------------

/// Fixture paths are deliberately real, resolvable targets even though nothing
/// here is compiled: `scripts/ci/check-include-str-paths.sh` scans every `.rs`
/// file under `crates/` for a literal `include_str!` call and requires its
/// target to exist on disk (its own, complementary rule — that scanner checks
/// *existence and Docker build-context coverage*, this one checks *crate-root
/// containment*). Fictional fixture paths would read to it as real breakage.
#[test]
fn reborn_include_literal_extraction_self_test() {
    let source = r#"
        const A: &str = include_str!("../CLAUDE.md");
        const B: &[u8] = include_bytes!(
            "../../ironclaw_common/Cargo.toml"
        );
        macro_rules! schema {
            ($path:literal) => {
                include_bytes!(concat!("../../ironclaw_common/src/", $path))
            };
        }
        const D: &str = include_str!(concat!(env!("OUT_DIR"), "/generated.rs"));
    "#;

    let literals = include_literals(source)
        .into_iter()
        .map(|(_, literal)| literal)
        .collect::<Vec<_>>();
    assert_eq!(
        literals,
        vec![
            "../CLAUDE.md".to_string(),
            "../../ironclaw_common/Cargo.toml".to_string(),
            "../../ironclaw_common/src/".to_string(),
        ],
        "the scanner must read single-line, rustfmt-split, and concat!-prefixed includes, and \
         must skip the env!-prefixed form rather than guess at it"
    );

    let lines = include_literals(source)
        .into_iter()
        .map(|(line, _)| line)
        .collect::<Vec<_>>();
    assert_eq!(
        lines,
        vec![2, 3, 8],
        "each finding must report the line the macro starts on"
    );
}

#[test]
fn reborn_include_escape_classifier_self_test() {
    let package_root = Path::new("/w/crates/ironclaw_example");

    // Negative fixtures: paths that stay inside the crate must not be flagged.
    for inside in [
        "../prompts/system.md",
        "./sibling.rs",
        "assets/manifest.toml",
        "../../ironclaw_example/assets/manifest.toml",
    ] {
        let resolved = resolve(&package_root.join("src/nested"), inside);
        assert!(
            contains(package_root, &resolved),
            "{inside} resolves to {} which is inside the crate root",
            resolved.display()
        );
    }

    // Positive fixtures: the two shapes §11.2.7 exists for.
    let into_sibling_crate = resolve(
        &package_root.join("src"),
        "../../ironclaw_first_party_extensions/assets/gmail/manifest.toml",
    );
    assert!(
        !contains(package_root, &into_sibling_crate),
        "a reach into a sibling crate must be flagged"
    );
    let into_repo_root = resolve(&package_root.join("src"), "../../../providers.json");
    assert!(
        !contains(package_root, &into_repo_root),
        "a reach into a repo-root asset must be flagged"
    );

    // And the owner attribution the report groups by.
    let packages = BTreeMap::from([
        ("ironclaw_example".to_string(), package_root.to_path_buf()),
        (
            "ironclaw_first_party_extensions".to_string(),
            PathBuf::from("/w/crates/ironclaw_first_party_extensions"),
        ),
        // The workspace-root package contains every crate and must never win.
        ("workspace_root_package".to_string(), PathBuf::from("/w")),
    ]);
    let root = Path::new("/w");
    assert_eq!(
        owning_package(&packages, root, &into_sibling_crate),
        Some("ironclaw_first_party_extensions")
    );
    assert_eq!(
        owning_package(&packages, root, &into_repo_root),
        None,
        "a repo-root asset has no owning crate — the workspace-root package must not match"
    );
}
