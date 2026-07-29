//! Zero-legacy gate for the memory lifecycle-capabilities rework (#3537).
//!
//! The rework retired a vocabulary: the `[memory]` operation families
//! (`MemoryOperationKind` / `operations = [...]` / the mandatory
//! `document_store` family), the single dual-lane `retrieve_context` provider
//! method (replaced by explicit `read_long_term` / `read_short_term` lane
//! methods), and the Rust-declared `builtin.profile_set` capability (now
//! `ironclaw.memory.profile_set`, declared by the bound provider's manifest).
//! This test pins all of it at **zero occurrences** across Reborn code
//! (`crates/`, including the WebUI frontend sources, and
//! `tests/integration/`) so none of it can be reintroduced silently — same
//! shape as `reborn_retired_taxonomy.rs`.
//!
//! Sanctioned exceptions are path-scoped, not term-scoped:
//! - the v1 gateway enclave is being strangled wholesale, not policed
//!   term-by-term;
//! - this test names every term on purpose.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates")
        .to_path_buf()
}

/// Retired memory vocabulary. A hit outside the sanctioned paths is a
/// regression, not a style issue.
const RETIRED_TERMS: &[&str] = &[
    // The operation-family enum (replaced by MemoryLifecycleHook).
    "MemoryOperationKind",
    // The dual-lane provider method (replaced by the explicit lane methods).
    "retrieve_context",
    // The `[memory].operations` manifest key (replaced by `lifecycle = [...]`
    // plus the provider's own `[[tools]]`).
    "operations = [",
    // The Rust-declared builtin profile tool (now the provider-declared
    // `ironclaw.memory.profile_set`).
    "builtin.profile_set",
    // The mandatory operation family (the `[[tools]]` array IS the
    // document-tool surface; no enum variant may mean "assume four tools").
    "document_store",
];

/// Path fragments allowed to reference retired vocabulary.
const SANCTIONED_PATHS: &[&str] = &[
    // The v1 gateway is a legacy enclave being strangled wholesale — not
    // policed term-by-term (same footing as `src/`).
    "crates/ironclaw_gateway/",
    // This gate names every term on purpose.
    "reborn_memory_retired_vocabulary.rs",
];

fn is_sanctioned(path: &str) -> bool {
    SANCTIONED_PATHS
        .iter()
        .any(|fragment| path.contains(fragment))
}

/// A scan error is a gate failure, not a skip: an unreadable directory or
/// file could hide a reintroduced term.
fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) -> std::io::Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits)?;
            continue;
        }
        let is_rust = name.ends_with(".rs");
        let is_frontend = name.ends_with(".ts")
            || name.ends_with(".tsx")
            || name.ends_with(".mts")
            || name.ends_with(".mjs")
            || name.ends_with(".js");
        let is_manifest = name.ends_with(".toml");
        if !(is_rust || is_frontend || is_manifest) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if is_sanctioned(&relative) {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| std::io::Error::new(error.kind(), format!("{relative}: {error}")))?;
        for term in RETIRED_TERMS {
            if contents.contains(term) {
                hits.push(format!("{relative}: `{term}`"));
            }
        }
    }
    Ok(())
}

#[test]
fn reborn_code_never_references_retired_memory_vocabulary() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits).expect("scan crates/ without I/O errors");
    scan_dir(&root, &root.join("tests/integration"), &mut hits)
        .expect("scan tests/integration without I/O errors");
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "retired memory vocabulary reintroduced (the bound provider's manifest \
         is the single source of truth: `[[tools]]` is the tool surface, \
         `[memory].lifecycle` gates every host-initiated call):\n{}",
        hits.join("\n")
    );
}
