//! Zero-legacy gate for the retired `ToolError::Failed` adapter variant.
//!
//! Adapter failures use the typed `ToolError::Rejected` shape at the extension
//! boundary and converge on the host API's canonical `DispatchError::Rejected`
//! path. Keeping a second adapter failure payload recreates the conversion and
//! provenance split this cleanup removes.
//!
//! Comments and string literals are exempt so this gate can explain the
//! migration. Only live Rust code is policed across `crates/` and `tests/`.

#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::{strip_comments_and_strings, workspace_root};

const RETIRED_TERM: &str = "ToolError::Failed";

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
        if relative.ends_with("reborn_tool_error_failed_retired.rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| std::io::Error::other(format!("read {relative}: {error}")))?;
        for (number, line) in strip_comments_and_strings(&contents).lines().enumerate() {
            if line.contains(RETIRED_TERM) {
                hits.push(format!("{relative}:{}: `{RETIRED_TERM}`", number + 1));
            }
        }
    }
    Ok(())
}

#[test]
fn reborn_code_never_constructs_or_matches_tool_error_failed() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits).expect("scan crates/");
    scan_dir(&root, &root.join("tests"), &mut hits).expect("scan tests/");
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "retired ToolError::Failed remains in live Rust code; use the canonical \
         ToolError::Rejected payload and host DispatchError::Rejected path:\n{}",
        hits.join("\n")
    );
}
