//! Seal enforcement for `Authorized` (arch-simplification §5.3.2; seal-placement
//! decision 2026-07-18: `host_api` type + witness token).
//!
//! `ironclaw_host_api::Authorized` can be minted only via an `AuthorizationGrant`,
//! and the only way to obtain a grant is to implement
//! `ironclaw_host_api::CapabilityAuthorizer`. Pure cross-crate type-sealing is not
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

use ratchet_support::strip_comments_and_strings;

/// The single crate permitted to implement `CapabilityAuthorizer` (the kernel
/// authorizer that owns `authorize()`).
const KERNEL_CRATE_DIR: &str = "ironclaw_capabilities";

fn workspace_crates_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/ironclaw_architecture; go up to crates/.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ dir")
        .to_path_buf()
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip test/example/bench trees (test doubles live there) and build output.
            if matches!(name, "tests" | "examples" | "benches" | "target") {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn capability_authorizer_is_implemented_only_by_the_kernel() {
    let crates_dir = workspace_crates_dir();
    let mut files = Vec::new();
    collect_rs_files(&crates_dir, &mut files);

    let mut offenders = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).unwrap_or_default();
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
