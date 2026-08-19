//! Zero-legacy gate for the retired lane-named `DispatchError` variants (PR 4
//! stage 1).
//!
//! `DispatchError::Mcp`, `DispatchError::Script`, `DispatchError::Wasm`, and
//! `DispatchError::FirstParty` carried no semantics distinct from the generic
//! `DispatchError::Rejected { runtime, kind, diagnostic, detail, attempt }`
//! shape — every consumer (`CapabilityInvocationError::from`,
//! `tool_error_from_dispatch` and `dispatch_error_for_tool_error` in
//! `ironclaw_extension_host`) fold them into the same fields. Provider
//! rejection is now uniformly `Rejected`, with the originating `RuntimeKind`
//! carried as metadata rather than encoded in the enum's variant tag.
//! `ironclaw_host_api::runtime::DispatchErrorLane` and
//! `RuntimeKind::dispatch_error_lane` — the classifier that routed a
//! `RuntimeKind` into one of the four lane variants — are deleted with them:
//! no consumer remains to classify into.
//!
//! This test pins the retired names at **zero live occurrences** across
//! `crates/` and `tests/` so the fold cannot quietly reverse into a second
//! per-lane variant.
//!
//! **Comments are exempt on purpose**, matching
//! `reborn_retired_failure_vocabulary.rs`: prose explaining what was retired
//! and why is worth keeping, so only code is policed.

#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// Retired `DispatchError` variant paths and the classifier that routed into
/// them. Qualified (`DispatchError::Wasm`, not bare `Wasm`) so the scan never
/// trips on the many legitimate, still-live uses of `RuntimeKind::Wasm`,
/// `RuntimeLane::Wasm`, the `wasm`/`mcp`/`script`/`first_party` runtime
/// strings, or the `FirstPartyCapabilityError`/`FirstPartyRuntimeAdapter`
/// types that remain.
const RETIRED_TERMS: &[&str] = &[
    "DispatchError::Mcp",
    "DispatchError::Script",
    "DispatchError::Wasm",
    "DispatchError::FirstParty",
    "DispatchErrorLane",
    "dispatch_error_lane",
];

/// Scan one directory tree, failing loudly on anything unreadable — a gate
/// that silently skips an unreadable path passes vacuously.
fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) -> std::io::Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| std::io::Error::other(format!("read_dir {}: {error}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            std::io::Error::other(format!("entry under {}: {error}", dir.display()))
        })?;
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
        if !name.ends_with(".rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // This gate names every retired term on purpose.
        if relative.ends_with("reborn_retired_dispatch_error_lanes.rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| std::io::Error::other(format!("read {relative}: {error}")))?;
        let stripped = strip_comments_and_strings(&contents);
        for (number, line) in stripped.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            for term in RETIRED_TERMS {
                if line.contains(term) {
                    hits.push(format!("{relative}:{}: `{term}`", number + 1));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn reborn_code_never_reintroduces_lane_named_dispatch_error_variants() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits).expect("scan crates/");
    scan_dir(&root, &root.join("tests"), &mut hits).expect("scan tests/");
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "a retired lane-named `DispatchError` variant (or its `DispatchErrorLane` \
         classifier) is back in live code — provider rejection is uniformly \
         `DispatchError::Rejected {{ runtime, kind, diagnostic, detail, attempt }}`, \
         with `runtime: Option<RuntimeKind>` carrying the originating lane as \
         metadata instead of a second per-lane enum variant:\n{}",
        hits.join("\n")
    );
}
