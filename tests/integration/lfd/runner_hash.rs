//! `meta.runner_hash` (SCHEMA.md §6): sha256 over the sorted `(path, sha256)`
//! list of every file under the pinned runner directories, computed at runtime
//! from the sources on disk. The scorer recomputes the same hash to verify the
//! runner was not tampered with between pinning and scoring.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The hash-pinned source trees: this runner module (INCLUDING
/// `profiles/<feature>.rs` — profile edits are visible in the audit trail even
/// though they are the one agent-writable file per feature), the shared
/// integration-harness support tree the runner drives, and the shared scorer
/// sources that interpret the emitted outcomes.
const PINNED_DIRS: [&str; 3] = [
    "lfd/_shared/scorer",
    "tests/integration/lfd",
    "tests/integration/support",
];

pub fn compute() -> Result<String, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut entries: Vec<(String, String)> = Vec::new();
    for dir in PINNED_DIRS {
        collect_files(&manifest_dir.join(dir), &manifest_dir, &mut entries)?;
    }
    entries.sort();
    // Byte-identical to the scorer's `combined_pin_hash` (lfd/_shared/scorer/
    // lint_core.py): sha256 over the newline-joined `path=sha256(file)` lines,
    // sorted by path — NO trailing newline, `=` separator. The scorer
    // recomputes this from `pins.json` and rejects any outcome whose
    // `meta.runner_hash` disagrees, so the two formulas must match exactly.
    let joined = entries
        .iter()
        .map(|(relative_path, file_hash)| format!("{relative_path}={file_hash}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(hex::encode(Sha256::digest(joined.as_bytes())))
}

fn collect_files(
    dir: &Path,
    manifest_dir: &Path,
    entries: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let read_dir = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read pinned dir {dir:?}: {error}"))?;
    for entry in read_dir {
        let entry = entry.map_err(|error| format!("cannot list pinned dir {dir:?}: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, manifest_dir, entries)?;
        } else {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("cannot read pinned file {path:?}: {error}"))?;
            let relative = path
                .strip_prefix(manifest_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            entries.push((relative, hex::encode(Sha256::digest(&bytes))));
        }
    }
    Ok(())
}
