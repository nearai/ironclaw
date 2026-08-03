//! Seal enforcement for `Authorized` (arch-simplification §5.3.2; seal-placement
//! decision 2026-07-18: `host_api` type + witness token).
//!
//! `ironclaw_host_api::authorized::Authorized` can be minted only via an `AuthorizationGrant`,
//! and the only way to obtain a grant is to implement
//! `ironclaw_host_api::authorized::CapabilityAuthorizer`. Pure cross-crate type-sealing is not
//! expressible in Rust (host_api defines the type; the kernel is the sole
//! legitimate minter), so this test supplies the other half of the seal: **only
//! the kernel crate may implement `CapabilityAuthorizer`.**
//!
//! If any other production crate implements it, that crate can forge an
//! `Authorized` and dispatch an un-authorized invocation — the exact security
//! property the seal exists to prevent. Test doubles belong under `tests/`
//! (skipped here), matching the sibling ratchets' convention.
//!
//! Definition of done: when `authorize()` is wired, the ONE production impl lives
//! in `ironclaw_capabilities`; this test keeps it the only one.

// Each integration-test binary compiles the shared module independently; this
// binary uses only the comment/string stripper, so the other shared helpers
// are dead code HERE (and live in the sibling ratchet binaries).
#[allow(dead_code)]
mod ratchet_support;

use std::fs;
use std::path::{Path, PathBuf};

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// The single crate permitted to implement `CapabilityAuthorizer` (the kernel
/// authorizer that owns `authorize()`).
const KERNEL_CRATE_DIR: &str = "ironclaw_capabilities";

/// Sanity floor for the production walk. Far below the real count (~1300) — it
/// is not a ratchet and is never expected to bind. It exists because this gate
/// used to resolve its scan root as `CARGO_MANIFEST_DIR.parent()`, i.e. one
/// level up, which under a family move lands on `crates/<family>` — a directory
/// that *exists*. The walk then quietly covered one family instead of the
/// workspace and the gate still passed: measured at 45 files instead of 1309 on
/// a partially-moved tree, with no error. A security gate that can pass having
/// scanned a fraction of the tree is the defect (CHECKLIST WS0, #6963).
const MIN_SCANNED_FILES: usize = 500;

/// Read a file the walk already found. An I/O failure fails the gate.
///
/// `read_to_string(..).unwrap_or_default()` was the shape at the call site, and
/// it is fail-open by construction: an unreadable file becomes an empty string,
/// contributes no offenders, and scans exactly like a clean one. The floor
/// below cannot see it — one unreadable file inside an otherwise healthy
/// workspace keeps the count far above 500 — so the seal would report "no rogue
/// implementors" for a file it never read (CHECKLIST WS0, #6963).
fn read_source(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read {} — a source this seal cannot read must fail the gate, not scan \
             as empty: {error}",
            path.display()
        )
    })
}

/// Walk `dir` for production `.rs` files. An unreadable directory or entry is an
/// error, not a silently skipped subtree: the floor only catches a scan that
/// went almost entirely dark, never one subtree vanishing from it.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| {
        format!(
            "failed to read {} — an unreadable directory must fail this walk, not vanish \
             from it: {error}",
            dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to read an entry in {}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip test/example/bench trees (test doubles live there), build
            // output, and vendored frontend packages (`crates/ironclaw_webui/
            // frontend/node_modules` after a pnpm install — never workspace
            // source, and it is not policed by this seal).
            if matches!(
                name,
                "tests" | "examples" | "benches" | "target" | "node_modules"
            ) {
                continue;
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Production Rust sources under `crates/`, refusing to return a set too small
/// to be the workspace. Returning the count keeps "did it measure anything?"
/// answerable by the caller rather than implied.
fn scanned_production_files(crates_dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !crates_dir.is_dir() {
        return Err(format!(
            "no crates/ directory at {} — the scan root is wrong, so this seal \
             would police nothing",
            crates_dir.display()
        ));
    }
    let mut files = Vec::new();
    collect_rs_files(crates_dir, &mut files)?;
    if files.len() < MIN_SCANNED_FILES {
        return Err(format!(
            "production walk under {} found only {} file(s) (floor {}). The walk is \
             broken, not the workspace — this seal is the only barrier to forging an \
             `Authorized`, and it must never pass having scanned a fraction of the tree.",
            crates_dir.display(),
            files.len(),
            MIN_SCANNED_FILES
        ));
    }
    Ok(files)
}

#[test]
fn capability_authorizer_is_implemented_only_by_the_kernel() {
    let crates_dir = workspace_root().join("crates");
    let files =
        scanned_production_files(&crates_dir).expect("production walk must measure the workspace");

    let mut offenders = Vec::new();
    for file in files {
        let source = read_source(&file).expect("every scanned source must be readable");
        // Comments and string literals are stripped (shared ratchet lexer), so
        // doc mentions and error-message text cannot false-positive — and a
        // qualified path (`impl crate::CapabilityAuthorizer for`) or a
        // multiline `impl<...>\n CapabilityAuthorizer for` header cannot evade
        // a starts_with("impl") check: any stripped line containing the
        // `CapabilityAuthorizer for` implementation head counts.
        let stripped = strip_comments_and_strings(&source);
        for raw in stripped.lines() {
            let line = raw.trim();
            if line.contains("CapabilityAuthorizer for") {
                let path = file.to_string_lossy().to_string();
                // Platform-agnostic containment check: `components()` avoids
                // hardcoding a separator (Windows paths use backslashes).
                let in_kernel = file
                    .components()
                    .any(|component| component.as_os_str() == KERNEL_CRATE_DIR);
                if !in_kernel {
                    offenders.push(format!("{path}: {line}"));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "`CapabilityAuthorizer` may be implemented ONLY in `{KERNEL_CRATE_DIR}` — it is the sole \
         legitimate minter of `AuthorizationGrant`/`Authorized` (arch-simplification §5.3.2). A \
         production impl elsewhere can forge an authorized invocation. Move test doubles under \
         `tests/`. Offending impls: {offenders:?}"
    );
}

/// The walk must refuse a scan root that cannot be the workspace. Both shapes
/// the family move produces are covered: a root with no `crates/` at all, and a
/// `crates/` holding one family's worth of files (the shape that used to pass).
#[test]
fn production_walk_refuses_a_scan_root_that_is_not_the_workspace() {
    let temp = tempfile::tempdir().expect("tempdir");

    let error = scanned_production_files(&temp.path().join("crates"))
        .expect_err("a missing crates/ directory must refuse");
    assert!(
        error.contains("no crates/ directory"),
        "expected a scan-root refusal, got: {error}"
    );

    // One family's worth of source: real files, real directory, far too few.
    let family = temp
        .path()
        .join("crates/substrates/ironclaw_capabilities/src");
    fs::create_dir_all(&family).expect("family tree");
    for index in 0..10 {
        fs::write(family.join(format!("m{index}.rs")), "pub fn f() {}\n").expect("source");
    }
    let error = scanned_production_files(&temp.path().join("crates"))
        .expect_err("a partial tree must refuse");
    assert!(
        error.contains("found only 10 file(s)"),
        "expected a measurement refusal naming the count, got: {error}"
    );
}

/// The floor detects only a scan that went almost entirely dark. It cannot see
/// a single unreadable directory or file inside an otherwise healthy workspace
/// — and that file could be the one holding a rogue `CapabilityAuthorizer`
/// impl. So the walk and the read both refuse, and both refusals are pinned
/// here with failures that are deterministic on every platform (`read_dir` on a
/// regular file; `read_to_string` on a directory) rather than a chmod that root
/// ignores inside a container.
#[test]
fn an_unreadable_path_fails_the_scan_rather_than_disappearing_from_it() {
    let temp = tempfile::tempdir().expect("tempdir");

    let not_a_directory = temp.path().join("regular-file");
    fs::write(&not_a_directory, "// fixture\n").expect("fixture file");
    let mut files = Vec::new();
    let error = collect_rs_files(&not_a_directory, &mut files)
        .expect_err("an unreadable directory must refuse");
    assert!(
        error.contains("must fail this walk, not vanish from it"),
        "expected a walk refusal naming the path, got: {error}"
    );

    let error = read_source(temp.path()).expect_err("an unreadable source must refuse");
    assert!(
        error.contains("must fail the gate, not scan as empty"),
        "expected a read refusal naming the path, got: {error}"
    );

    // And the refusal reaches the gate's own entry point, not just the helper.
    let crates_dir = temp.path().join("crates");
    fs::create_dir_all(&crates_dir).expect("crates dir");
    fs::write(crates_dir.join("not-a-crate-dir"), "x").expect("file");
    let error = scanned_production_files(&crates_dir.join("not-a-crate-dir"))
        .expect_err("a scan root that is not a directory must refuse");
    assert!(
        error.contains("no crates/ directory"),
        "expected the scan-root refusal, got: {error}"
    );
}

/// The real workspace clears the floor with room to spare — the floor is a
/// sanity check, not a ratchet that will bind on ordinary growth or deletion.
#[test]
fn production_walk_measures_the_whole_workspace() {
    let files = scanned_production_files(&workspace_root().join("crates"))
        .expect("the real workspace must clear the floor");
    assert!(
        files.len() > MIN_SCANNED_FILES * 2,
        "the floor is meant to sit far below reality; scanned {} with floor {}",
        files.len(),
        MIN_SCANNED_FILES
    );
    assert!(
        files
            .iter()
            .any(|path| path.components().any(|c| c.as_os_str() == KERNEL_CRATE_DIR)),
        "the walk must reach the kernel crate it polices"
    );
}

/// The retired one-level root landed on `crates/<family>` under a family move —
/// an existing directory — so the walk silently narrowed instead of failing.
#[test]
fn workspace_root_search_survives_a_family_move() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("workspace manifest");
    let nested = root.join("crates/substrates/ironclaw_architecture");
    fs::create_dir_all(&nested).expect("nested crate");

    let found = ratchet_support::find_workspace_root(&nested).expect("root found");
    assert_eq!(
        found.canonicalize().ok(),
        root.canonicalize().ok(),
        "the search must find the workspace root from a family-nested crate"
    );

    // The retired rule: one level up from the nested crate is `crates/substrates`,
    // which exists — so the old walk covered one family and reported success.
    let retired = nested.parent().map(Path::to_path_buf);
    assert_eq!(retired, Some(root.join("crates/substrates")));
    assert!(
        retired.expect("retired root").is_dir(),
        "the retired scan root existed, which is exactly why the narrowing was silent"
    );

    assert!(
        ratchet_support::find_workspace_root(Path::new("/")).is_err(),
        "a path with no workspace ancestor must refuse, not guess"
    );
}

#[test]
fn seal_ratchet_self_test_detects_a_non_kernel_impl() {
    // Guards the matcher itself: the check must fire on an impl outside the kernel.
    let sample_line = "impl CapabilityAuthorizer for RogueMinter {}";
    let matches =
        sample_line.trim().starts_with("impl") && sample_line.contains("CapabilityAuthorizer for");
    assert!(matches, "matcher must detect a bare non-kernel impl");
    // And must NOT fire on the trait definition or a comment.
    for benign in [
        "pub trait CapabilityAuthorizer {",
        "// impl CapabilityAuthorizer for Foo (in docs)",
        "/// See CapabilityAuthorizer for details",
    ] {
        let t = benign.trim();
        let fires =
            !t.starts_with("//") && t.starts_with("impl") && t.contains("CapabilityAuthorizer for");
        assert!(!fires, "matcher must not fire on: {benign}");
    }
}
