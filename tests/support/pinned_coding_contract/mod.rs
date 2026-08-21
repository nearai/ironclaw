//! Test-only support for the pinned core-tool contract snapshot.
//!
//! The snapshot at `tests/fixtures/pinned_coding_contract/` pins the exact
//! model-visible contract of the eight pinned core coding tools (`read`,
//! `write`, `edit`, `glob`, `grep`, `bash`, `ast_grep`, `ast_edit`) at upstream
//! commit [`PINNED_COMMIT`] of `can1357/oh-my-pi`:
//!
//! - `manifest.json` — the eight-tool inventory and per-tool contract mapping
//!   (schema, prompt, grammar, selector/error/output fixtures, exact required
//!   case-ID inventories, the rendered read prompt record).
//! - `provenance.json` — the pinned commit, MIT license record, and a
//!   per-file SHA-256 record for every contract-defining upstream file;
//!   snapshotted files are byte-identical verbatim copies. Offline byte
//!   verification covers the vendored and derived assets plus `manifest.json`
//!   (via its provenance record); unsnapshotted upstream records are
//!   capture-time pins and are not byte-verified offline.
//! - `licenses/LICENSE` — the vendored full upstream MIT license text.
//! - `schemas/*.json` — the rendered model-visible wire schemas (rendered
//!   through the pinned upstream omptype/pi-ai `toolWireSchema` pipeline).
//! - `prompts/*.md`, `grammars/*.lark`, `sources/*.ts` — verbatim upstream
//!   prompt assets, Hashline/apply_patch grammars, and the small selector /
//!   output / error contract sources; `prompts/read.rendered.md` is the
//!   derived fully rendered read description for the pinned issue-target
//!   context.
//! - `golden/selectors.json` — deterministic selector-parse cases generated
//!   from the pinned upstream parser.
//! - `golden/errors/` and `golden/output/` — representative model-visible
//!   error shapes and output-format examples transcribed verbatim from the
//!   pinned upstream sources (shape pins, not a runtime oracle).
//!
//! Everything loads from the checked-in fixture tree; tests never touch the
//! network. The accessors below are the intended reuse seam for the later
//! pinned-vs-IronClaw differential execution tests (issue #7392): load the
//! snapshot once, then drive both implementations with the same
//! schema/prompt/selector/error cases.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The reviewed upstream commit this snapshot is pinned to (issue #7392).
pub const PINNED_COMMIT: &str = "08819b279cf02ae2545e69dad7111ab48d91d35e";

/// The exact eight-tool inventory, in canonical order.
pub const EXPECTED_TOOL_NAMES: [&str; 8] = [
    "read", "write", "edit", "glob", "grep", "bash", "ast_grep", "ast_edit",
];

/// Absolute path of the checked-in snapshot fixture tree.
pub fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pinned_coding_contract")
}

/// Read a file inside the snapshot tree; panics with the path on any I/O error.
pub fn read_snapshot_file(relative_path: &str) -> Vec<u8> {
    let path = fixture_root().join(relative_path);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("cannot read snapshot file {path:?}: {error}"))
}

/// Read a UTF-8 text file inside the snapshot tree.
pub fn read_snapshot_text(relative_path: &str) -> String {
    let bytes = read_snapshot_file(relative_path);
    String::from_utf8(bytes)
        .unwrap_or_else(|error| panic!("snapshot file {relative_path} is not UTF-8: {error}"))
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamRecord {
    pub repository: String,
    pub commit: String,
    pub license: String,
    /// Snapshot-relative path of the vendored full upstream license text
    /// (checked in at `licenses/LICENSE`); see also `license_upstream_path`.
    #[serde(default)]
    pub license_file: Option<String>,
    /// Upstream path of the license file (relative to the repository root).
    #[serde(default)]
    pub license_upstream_path: Option<String>,
    #[serde(default)]
    pub license_notice: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileRecord {
    /// Upstream path relative to the repository root.
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    pub role: String,
    #[serde(default)]
    pub snapshotted: bool,
    /// Path relative to the snapshot root when the file is vendored.
    #[serde(default)]
    pub snapshot_path: Option<String>,
}

/// A fixture file derived from the pinned upstream contract (rendered schemas,
/// golden selector/error/output cases, the rendered read prompt), checksummed
/// like the vendored files.
#[derive(Debug, Clone, Deserialize)]
pub struct DerivedRecord {
    /// Path relative to the snapshot root.
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    pub role: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// The checksum record for the snapshot inventory file `manifest.json` itself.
/// `provenance.json` is the only metadata file exempt from checksum coverage.
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestRecord {
    /// Path relative to the snapshot root (`manifest.json`).
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Provenance {
    pub schema_version: u32,
    pub upstream: UpstreamRecord,
    pub files: Vec<FileRecord>,
    #[serde(default)]
    pub derived: Vec<DerivedRecord>,
    #[serde(default)]
    pub manifest: Option<ManifestRecord>,
}

/// A prompt template rendered for a pinned context (currently only `read`).
#[derive(Debug, Clone, Deserialize)]
pub struct RenderedPrompt {
    /// Path of the rendered asset relative to the snapshot root.
    pub path: String,
    /// The exact render context the asset was produced with.
    #[serde(default)]
    pub context: Value,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolEntry {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub schema: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub grammar: Option<String>,
    #[serde(default)]
    pub selectors_source: Option<String>,
    #[serde(default)]
    pub errors_fixture: Option<String>,
    /// Exact required error-case IDs for the tool's error fixture.
    #[serde(default)]
    pub errors_case_ids: Vec<String>,
    #[serde(default)]
    pub output_fixture: Option<String>,
    /// Exact required output-case IDs for the tool's output fixture (null when
    /// the tool has no output fixture).
    #[serde(default)]
    pub output_case_ids: Option<Vec<String>>,
    /// A prompt template rendered for a pinned context, when one is checked in.
    #[serde(default)]
    pub rendered_prompt: Option<RenderedPrompt>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub pinned_commit: String,
    pub tool_names: Vec<String>,
    /// Exact required selector-case IDs for `golden/selectors.json`.
    #[serde(default)]
    pub selector_case_ids: Vec<String>,
    pub tools: BTreeMap<String, ToolEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorCase {
    pub sel: String,
    #[serde(default)]
    pub selector: Option<Value>,
    #[serde(default)]
    pub offset_limit: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEntry {
    pub case: String,
    pub kind: String,
    pub template: String,
    #[serde(default)]
    pub example: Option<String>,
    pub source_path: String,
    #[serde(default)]
    pub source_line: Option<u64>,
}

/// Load and parse `manifest.json` from the checked-in snapshot.
pub fn load_manifest() -> Manifest {
    let text = read_snapshot_text("manifest.json");
    serde_json::from_str(&text).expect("pinned contract manifest.json must parse")
}

/// Load and parse `provenance.json` from the checked-in snapshot.
pub fn load_provenance() -> Provenance {
    let text = read_snapshot_text("provenance.json");
    serde_json::from_str(&text).expect("pinned contract provenance.json must parse")
}

/// The rendered model-visible JSON input schema for `tool`.
pub fn tool_schema(tool: &str) -> Value {
    let entry = tool_entry(tool);
    let text = read_snapshot_text(&entry.schema);
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("schema for {tool} must parse: {error}"))
}

/// The verbatim upstream prompt asset for `tool`.
pub fn tool_prompt(tool: &str) -> String {
    let entry = tool_entry(tool);
    let prompt = entry
        .prompt
        .as_ref()
        .unwrap_or_else(|| panic!("tool {tool} has no prompt asset"));
    read_snapshot_text(prompt)
}

/// The deterministic selector-parse cases for the read selector grammar.
pub fn selector_cases() -> Vec<SelectorCase> {
    let text = read_snapshot_text("golden/selectors.json");
    serde_json::from_str(&text).expect("golden/selectors.json must parse")
}

/// Representative model-visible error entries for `tool`.
pub fn error_entries(tool: &str) -> Vec<ErrorEntry> {
    let entry = tool_entry(tool);
    let fixture = entry
        .errors_fixture
        .as_ref()
        .unwrap_or_else(|| panic!("tool {tool} has no errors fixture"));
    let text = read_snapshot_text(fixture);
    let value: Value = serde_json::from_str(&text).expect("error fixture must parse");
    serde_json::from_value(
        value
            .get("entries")
            .cloned()
            .expect("error fixture has entries"),
    )
    .expect("error fixture entries must deserialize")
}

/// The vendored full upstream MIT license text (checked-in asset recorded in
/// provenance.json under `upstream.license_file`).
pub fn license_text() -> String {
    let provenance = load_provenance();
    let path = provenance
        .upstream
        .license_file
        .as_deref()
        .expect("upstream.license_file must point at the vendored license asset");
    read_snapshot_text(path)
}

/// The rendered model-visible description for `tool` when a pinned render
/// context is recorded in the manifest (currently only `read`); `None` for
/// tools whose prompt is a verbatim template with no pinned render.
pub fn rendered_tool_prompt(tool: &str) -> Option<String> {
    let entry = tool_entry(tool);
    entry
        .rendered_prompt
        .map(|rendered| read_snapshot_text(&rendered.path))
}

fn tool_entry(tool: &str) -> ToolEntry {
    let manifest = load_manifest();
    manifest
        .tools
        .get(tool)
        .cloned()
        .unwrap_or_else(|| panic!("tool {tool} missing from manifest"))
}

/// Recompute the SHA-256 of every vendored upstream file, every derived
/// fixture file, and `manifest.json` (via its provenance record), returning
/// the mismatches as `(snapshot_path, recorded, actual)`; empty when all
/// match. Unsnapshotted upstream records are capture-time pins and are not
/// byte-verified offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumMismatch {
    Sha256 {
        path: String,
        recorded: String,
        actual: String,
    },
    ByteCount {
        path: String,
        recorded: usize,
        actual: usize,
    },
}

impl std::fmt::Display for ChecksumMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sha256 {
                path,
                recorded,
                actual,
            } => {
                write!(
                    formatter,
                    "{path}: SHA-256 recorded {recorded}, actual {actual}"
                )
            }
            Self::ByteCount {
                path,
                recorded,
                actual,
            } => {
                write!(
                    formatter,
                    "{path}: bytes recorded {recorded}, actual {actual}"
                )
            }
        }
    }
}

pub fn verify_snapshotted_checksums(provenance: &Provenance) -> Vec<ChecksumMismatch> {
    let mut mismatches = Vec::new();
    for record in &provenance.files {
        if !record.snapshotted {
            continue;
        }
        let snapshot_path = record
            .snapshot_path
            .as_deref()
            .unwrap_or_else(|| panic!("snapshotted record {} lacks snapshot_path", record.path));
        verify_checksum(&mut mismatches, snapshot_path, &record.sha256, record.bytes);
    }
    for record in &provenance.derived {
        verify_checksum(&mut mismatches, &record.path, &record.sha256, record.bytes);
    }
    if let Some(record) = &provenance.manifest {
        verify_checksum(&mut mismatches, &record.path, &record.sha256, record.bytes);
    }
    mismatches
}

fn verify_checksum(
    mismatches: &mut Vec<ChecksumMismatch>,
    path: &str,
    recorded: &str,
    recorded_bytes: usize,
) {
    let bytes = read_snapshot_file(path);
    let actual = sha256_hex(&bytes);
    if actual != recorded {
        mismatches.push(ChecksumMismatch::Sha256 {
            path: path.to_string(),
            recorded: recorded.to_string(),
            actual,
        });
    }
    if bytes.len() != recorded_bytes {
        mismatches.push(ChecksumMismatch::ByteCount {
            path: path.to_string(),
            recorded: recorded_bytes,
            actual: bytes.len(),
        });
    }
}

/// Files present under the snapshot root that no provenance record references.
/// `provenance.json` is the only self-exempt metadata file: it cannot carry
/// its own checksum; `manifest.json` is covered by its own provenance record.
pub fn orphan_snapshot_files(provenance: &Provenance) -> Vec<String> {
    let mut referenced: BTreeMap<String, ()> = BTreeMap::new();
    for record in &provenance.files {
        if let Some(snapshot_path) = &record.snapshot_path {
            referenced.insert(snapshot_path.clone(), ());
        }
    }
    for record in &provenance.derived {
        referenced.insert(record.path.clone(), ());
    }
    if let Some(record) = &provenance.manifest {
        referenced.insert(record.path.clone(), ());
    }
    let mut orphans = Vec::new();
    collect_orphans(&fixture_root(), "", &referenced, &mut orphans);
    orphans
}

/// Outcome of running one implementation on one case.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    /// The implementation produced a value (structured JSON output).
    Ok(Value),
    /// The implementation failed; the message carries the error text.
    Err(String),
}

/// A structured disagreement between the baseline and candidate
/// implementations for a single named case.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseMismatch {
    /// Stable case identifier (selector string, error case id, ...).
    pub case: String,
    pub baseline: RunOutcome,
    pub candidate: RunOutcome,
}

/// Run the `baseline` and `candidate` implementations over every named case
/// and return the structured mismatches; empty when both agree on all cases.
///
/// This is the reusable old-vs-new comparison seam for the later
/// pinned-vs-IronClaw differential execution tests (issue #7392): feed it the
/// checked-in cases (selector/error/output fixtures) with the two real tool
/// engines as closures. It is deliberately mock-free and engine-agnostic.
pub fn compare_cases<C, F, G>(
    cases: &[C],
    case_name: impl Fn(usize, &C) -> String,
    baseline: F,
    candidate: G,
) -> Vec<CaseMismatch>
where
    F: Fn(&C) -> Result<Value, String>,
    G: Fn(&C) -> Result<Value, String>,
{
    let mut mismatches = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let name = case_name(index, case);
        let baseline = match baseline(case) {
            Ok(value) => RunOutcome::Ok(value),
            Err(message) => RunOutcome::Err(message),
        };
        let candidate = match candidate(case) {
            Ok(value) => RunOutcome::Ok(value),
            Err(message) => RunOutcome::Err(message),
        };
        if baseline != candidate {
            mismatches.push(CaseMismatch {
                case: name,
                baseline,
                candidate,
            });
        }
    }
    mismatches
}

fn collect_orphans(
    dir: &Path,
    relative: &str,
    referenced: &BTreeMap<String, ()>,
    orphans: &mut Vec<String>,
) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("cannot read {dir:?}: {error}"));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let child_relative = if relative.is_empty() {
            file_name.clone()
        } else {
            format!("{relative}/{file_name}")
        };
        if entry.file_type().expect("file type").is_dir() {
            collect_orphans(&entry.path(), &child_relative, referenced, orphans);
        } else if child_relative != "provenance.json" && !referenced.contains_key(&child_relative) {
            orphans.push(child_relative);
        }
    }
}
