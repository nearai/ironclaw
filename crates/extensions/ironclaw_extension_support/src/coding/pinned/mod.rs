//! Pinned coding engines (issue #7392).
//!
//! These engines implement the model-visible contract of six pinned coding
//! tools (`read`, `write`, `edit`, `glob`, `grep`, `bash`) at upstream commit
//! `08819b279cf02ae2545e69dad7111ab48d91d35e` of `can1357/oh-my-pi`, backed by
//! [`RootFilesystem`] and the host's scoped artifact reader. The always-on
//! first-party package wires them through normal production dispatch. Contract
//! tests compare them with the pinned snapshot under
//! `tests/fixtures/pinned_coding_contract/`.
//!
//! Exact strings (selector errors, stale-anchor messages, output formats,
//! success shapes) are copied verbatim from the pinned upstream sources;
//! never approximate them.
//!
//! Scope notes: `artifact://` reads are implemented. Archives, SQLite,
//! documents, URLs, SSH, ast_grep/ast_edit, networked tools, and the
//! multi-backend conformance suite remain later issue #7392 slices.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{ids::RunId, mount::MountView, resource::ResourceScope};
use serde_json::{Value, json};

mod bash;
mod glob;
mod grep;
mod hashline;
/// Public surface for the pinned registration assets (issue #7392 slice 3):
/// the model-visible descriptions and input schemas, embedded in this crate
/// and exposed so downstream crates resolve them without cross-crate
/// `include_str!` reach-ins.
pub mod pinned_assets;
mod read;
mod selector;
mod state;
mod write;

pub use state::CodingSnapshotRegistry;

/// Stable classification of a pinned coding engine failure. The rendered message
/// ([`CodingEngineError::message`]) is the model-visible contract and is always
/// the exact pinned upstream text; the kind is a stable tag for tests and
/// callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingEngineErrorKind {
    /// Input did not satisfy the pinned wire schema (required field/type).
    Input,
    /// `read`/`glob`/`grep` path did not exist.
    PathNotFound,
    /// `read` selector could not be parsed (invalid selector text).
    InvalidSelector,
    /// Multi-range selector on a directory listing.
    MultiRangeDirectory,
    /// `glob` refused the filesystem root (`/`).
    RootNotAllowed,
    /// `glob` path list contained no non-empty entry.
    EmptyPath,
    /// `grep` pattern failed to compile.
    InvalidRegex,
    /// `grep` pattern was blank.
    PatternEmpty,
    /// `grep` skip was negative or non-finite.
    SkipNegative,
    /// `grep` line-range selector applied to a glob (not a single file).
    LineRangeSelectorRequiresSingleFile,
    /// `grep` line-range selector path did not exist.
    LineRangePathNotFound,
    /// `grep` line-range selector named a directory.
    LineRangeTargetIsDirectory,
    /// `grep` multi-path input: every entry missing.
    PathNotFoundMulti,
    /// `write` target looked URI-like but was not a known scheme.
    UnknownUriLikeTarget,
    /// `edit`: the section tag was recorded this run but the live file no
    /// longer hashes to it (stale anchor, hash recognized).
    StaleAnchorHashRecognized,
    /// `edit`: the section tag was never recorded for this scope+run (not
    /// from this session).
    StaleAnchorHashUnrecognized,
    /// `edit`: a line reference was malformed.
    MalformedLineReference,
    /// `edit`: an anchor referenced a line past EOF.
    LineOutOfBounds,
    /// `edit`: an absolute range endpoint was invalid.
    InvalidAbsoluteRange,
    /// `edit`: parse/apply failure with a pinned hashline message.
    HashlineApply,
    /// `edit`: multi-section aggregate failure.
    MultiEntryAggregate,
    /// Path did not resolve inside an authorized mount (IronClaw-specific;
    /// the pinned upstream tools run on the process filesystem and have no
    /// counterpart).
    PathResolution,
    /// The backing filesystem reported an error.
    Filesystem,
    /// Filesystem metadata or mount permissions denied access.
    FilesystemDenied,
    /// A bounded traversal or materialization limit was exceeded.
    ResourceLimit,
    /// Internal invariant violated (defensive; not a model-visible shape).
    Internal,
}

/// Failure of one pinned coding engine call. `message` is the exact
/// rendered pinned error text (pinned sources/fixtures); `kind` is a stable
/// classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingEngineError {
    kind: CodingEngineErrorKind,
    message: String,
}

impl CodingEngineError {
    pub(crate) fn new(kind: CodingEngineErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CodingEngineErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for CodingEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodingEngineError {}

/// Shared engine context: the backing filesystem, the mount view that
/// authorizes scoped paths, the caller scope (mirrors `coding/state.rs`
/// scope dimensions), the loop run identity, the bounded snapshot
/// registry that binds hashline edit tags to reads from the SAME run, and
/// the placement-neutral command executor used by the pinned `bash` engine
/// (absent when the composition selected no process backend).
#[derive(Clone)]
pub struct CodingEngineContext {
    pub filesystem: Arc<dyn RootFilesystem>,
    pub artifact_reader: Option<Arc<dyn ironclaw_host_api::artifact::ScopedArtifactReader>>,
    pub mounts: MountView,
    pub scope: ResourceScope,
    pub run_id: Option<RunId>,
    pub snapshots: Arc<CodingSnapshotRegistry>,
    pub process: Option<Arc<dyn ironclaw_host_api::process::CommandExecutor>>,
}

/// A resolved engine target: the canonical virtual path on the backend and
/// the granting mount.
pub(crate) struct ResolvedCodingPath {
    pub(crate) virtual_path: ironclaw_host_api::path::VirtualPath,
    pub(crate) grant: ironclaw_host_api::mount::MountGrant,
}

impl ResolvedCodingPath {
    /// Whether this resolution IS its grant's mount root (mirrors
    /// `coding/types.rs::ResolvedPath::is_mount_root`): a mount root the
    /// caller is authorized for exists by definition, so reads of the root
    /// itself behave as an empty directory rather than `NotFound`.
    pub(crate) fn is_mount_root(&self) -> bool {
        self.virtual_path.as_str() == self.grant.target.as_str()
    }
}

/// The six pinned engine entry points. Each returns the exact model-visible
/// output as JSON (`{"output": "<text>"}`) or the exact pinned error
/// text in [`CodingEngineError`].
pub async fn read(ctx: &CodingEngineContext, input: Value) -> Result<Value, CodingEngineError> {
    let output = read::read(ctx, input).await?;
    Ok(json!({ "output": output }))
}

pub async fn write(ctx: &CodingEngineContext, input: Value) -> Result<Value, CodingEngineError> {
    let output = write::write(ctx, input).await?;
    Ok(json!({ "output": output }))
}

pub async fn edit(ctx: &CodingEngineContext, input: Value) -> Result<Value, CodingEngineError> {
    let output = hashline::edit(ctx, input).await?;
    Ok(json!({ "output": output }))
}

/// Return the first file section's scoped path from a validated hashline edit.
///
/// The host uses this deterministic path as the working-directory hint for
/// its single advisory post-edit check. The edit engine has already parsed the
/// same input successfully before the host calls this helper.
pub fn first_edit_target_path(input: &Value) -> Option<String> {
    let edit = input.get("input")?.as_str()?;
    hashline::Patch::parse(edit)
        .ok()?
        .sections
        .first()
        .map(|section| scoped_path_input(&section.path))
}

pub async fn glob(ctx: &CodingEngineContext, input: Value) -> Result<Value, CodingEngineError> {
    let output = glob::glob(ctx, input).await?;
    Ok(json!({ "output": output }))
}

pub async fn grep(ctx: &CodingEngineContext, input: Value) -> Result<Value, CodingEngineError> {
    let output = grep::grep(ctx, input).await?;
    Ok(json!({ "output": output }))
}

/// The pinned `bash` engine. Output is a plain text render (OMP notices
/// included); the host adapter attaches exit-code metadata and artifact
/// spillage, mirroring the other coding engines' `{"output": ...}` shape.
pub async fn bash(ctx: &CodingEngineContext, input: Value) -> Result<Value, CodingEngineError> {
    let output = bash::bash(ctx, input).await?;
    Ok(json!({ "output": output }))
}

/// Public test seam for the root harness bin (`tests/reborn_coding_engines.rs`)
/// and the differential comparison factory: exposes the pinned selector
/// parser and error-template render functions without exposing engine
/// internals. Not part of any production surface.
#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub mod harness {
    use super::selector::{ParsedSelector, parse_sel, sel_to_offset_limit};
    use super::{CodingEngineErrorKind, hashline};
    use serde_json::{Value, json};

    /// Parse a read-tool selector and render the golden-shaped record
    /// `{"selector": …, "offset_limit": …}`, or return the exact pinned
    /// error text.
    pub fn parse_selector(sel: &str) -> Result<Value, String> {
        let parsed = parse_sel(Some(sel))?;
        let selector = match &parsed {
            ParsedSelector::None => json!({ "kind": "none" }),
            ParsedSelector::Raw => json!({ "kind": "raw" }),
            ParsedSelector::Conflicts => json!({ "kind": "conflicts" }),
            ParsedSelector::Lines { ranges, raw } => {
                let ranges: Vec<Value> = ranges
                    .iter()
                    .map(|range| {
                        let mut value = json!({ "startLine": range.start_line });
                        if let Some(end) = range.end_line {
                            value["endLine"] = json!(end);
                        }
                        value
                    })
                    .collect();
                let mut value = json!({ "kind": "lines", "ranges": ranges });
                if *raw {
                    value["raw"] = json!(true);
                }
                value
            }
        };
        let (offset, limit) = sel_to_offset_limit(&parsed);
        let offset_limit = match (offset, limit) {
            (Some(offset), Some(limit)) => json!({ "offset": offset, "limit": limit }),
            (Some(offset), None) => json!({ "offset": offset }),
            (None, _) => json!({}),
        };
        Ok(json!({ "selector": selector, "offset_limit": offset_limit }))
    }

    /// Render the stale-anchor rejection with the pinned recognized /
    /// not-from-session wording.
    pub fn render_stale_anchor(
        path: Option<&str>,
        expected: &str,
        actual: &str,
        file_lines: &[String],
        anchor_lines: &[u64],
        recognized: bool,
    ) -> String {
        hashline::render_mismatch_message(
            path,
            expected,
            actual,
            file_lines,
            anchor_lines,
            recognized,
        )
    }

    pub fn render_malformed_line_reference(raw: &str) -> String {
        hashline::malformed_line_reference(raw)
    }

    pub fn render_line_out_of_bounds(line: u64, line_count: usize) -> String {
        hashline::line_out_of_bounds(line, line_count)
    }

    /// Render the source-faithful invalid-absolute-range message (the
    /// fixture template pins the leading sentence; the engine extends it
    /// with the counted-range retry sentence, matching the pinned source).
    pub fn render_invalid_absolute_range(patch_line: u64, start: u64, end: u64) -> String {
        hashline::invalid_absolute_range_message(
            patch_line,
            start,
            end,
            hashline::AbsoluteRangeOp::Replace,
            None,
            None,
        )
    }

    pub fn render_per_file_failure(path: &str, error_text: &str) -> String {
        hashline::render_per_file_failure_aggregate(path, error_text)
    }

    pub fn render_files_not_applied(skipped_paths: &str) -> String {
        hashline::render_files_not_applied(skipped_paths)
    }

    pub fn render_auto_piped_warning() -> String {
        hashline::BARE_BODY_AUTO_PIPED_WARNING.to_string()
    }

    /// Compute the pinned hashline snapshot tag for `text` (xxHash32 low 16
    /// bits rendered as 4 uppercase hex digits, after the pinned
    /// normalization). The registration-seam integration test authors an
    /// `edit` payload with the tag of its seeded file BEFORE the scripted
    /// `read` result arrives, so it needs the same deterministic tag the
    /// engine will advertise.
    pub fn compute_file_hash(text: &str) -> String {
        hashline::format::compute_file_hash(text)
    }

    pub fn render_unknown_uri_like_target(trimmed: &str, suggestion: &str) -> String {
        super::write::render_unknown_uri_like_target(trimmed, suggestion)
    }

    /// Stable kind name for a rendered error (harness assertions).
    pub fn kind_name(kind: CodingEngineErrorKind) -> &'static str {
        match kind {
            CodingEngineErrorKind::Input => "Input",
            CodingEngineErrorKind::PathNotFound => "PathNotFound",
            CodingEngineErrorKind::InvalidSelector => "InvalidSelector",
            CodingEngineErrorKind::MultiRangeDirectory => "MultiRangeDirectory",
            CodingEngineErrorKind::RootNotAllowed => "RootNotAllowed",
            CodingEngineErrorKind::EmptyPath => "EmptyPath",
            CodingEngineErrorKind::InvalidRegex => "InvalidRegex",
            CodingEngineErrorKind::PatternEmpty => "PatternEmpty",
            CodingEngineErrorKind::SkipNegative => "SkipNegative",
            CodingEngineErrorKind::LineRangeSelectorRequiresSingleFile => {
                "LineRangeSelectorRequiresSingleFile"
            }
            CodingEngineErrorKind::LineRangePathNotFound => "LineRangePathNotFound",
            CodingEngineErrorKind::LineRangeTargetIsDirectory => "LineRangeTargetIsDirectory",
            CodingEngineErrorKind::PathNotFoundMulti => "PathNotFoundMulti",
            CodingEngineErrorKind::UnknownUriLikeTarget => "UnknownUriLikeTarget",
            CodingEngineErrorKind::StaleAnchorHashRecognized => "StaleAnchorHashRecognized",
            CodingEngineErrorKind::StaleAnchorHashUnrecognized => "StaleAnchorHashUnrecognized",
            CodingEngineErrorKind::MalformedLineReference => "MalformedLineReference",
            CodingEngineErrorKind::LineOutOfBounds => "LineOutOfBounds",
            CodingEngineErrorKind::InvalidAbsoluteRange => "InvalidAbsoluteRange",
            CodingEngineErrorKind::HashlineApply => "HashlineApply",
            CodingEngineErrorKind::MultiEntryAggregate => "MultiEntryAggregate",
            CodingEngineErrorKind::PathResolution => "PathResolution",
            CodingEngineErrorKind::Filesystem => "Filesystem",
            CodingEngineErrorKind::FilesystemDenied => "FilesystemDenied",
            CodingEngineErrorKind::ResourceLimit => "ResourceLimit",
            CodingEngineErrorKind::Internal => "Internal",
        }
    }
}

pub(crate) fn coding_error(
    kind: CodingEngineErrorKind,
    message: impl Into<String>,
) -> CodingEngineError {
    CodingEngineError::new(kind, message)
}

pub(crate) fn input_error(message: impl Into<String>) -> CodingEngineError {
    CodingEngineError::new(CodingEngineErrorKind::Input, message)
}

pub(crate) fn filesystem_denied() -> CodingEngineError {
    CodingEngineError::new(
        CodingEngineErrorKind::FilesystemDenied,
        "workspace file access denied",
    )
}

pub(crate) fn read_limit_exceeded() -> CodingEngineError {
    CodingEngineError::new(
        CodingEngineErrorKind::ResourceLimit,
        "workspace file exceeds the read limit",
    )
}

// ─── Path resolution ─────────────────────────────────────────────────────────
//
// Mirrors the sequence in `coding/paths.rs::resolve_path` (scoped-path
// normalization → mount resolution → sensitivity checks → permission gate),
// reusing its primitives where they are `pub(super)`-visible, but rendering
// failures as `CodingEngineError` instead of the dispatch-oriented
// `CodingCapabilityError`. The pinned upstream tools resolve against the process
// cwd; the IronClaw equivalent root is the workspace mount root
// (`DEFAULT_SCOPED_ROOT` alias).

const DEFAULT_SCOPED_ROOT: &str = "/workspace";

fn scoped_path_input(path: &str) -> String {
    if path == "." || path.is_empty() {
        DEFAULT_SCOPED_ROOT.to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else if let Some(scoped_workspace_path) = workspace_scoped_alias(path) {
        scoped_workspace_path
    } else {
        let relative = path.trim_start_matches("./");
        format!("{DEFAULT_SCOPED_ROOT}/{relative}")
    }
}

fn workspace_scoped_alias(path: &str) -> Option<String> {
    let path = strip_leading_current_dir_segments(path);
    if path == "workspace" {
        return Some(DEFAULT_SCOPED_ROOT.to_string());
    }
    path.strip_prefix("workspace/")
        .map(|relative| relative.trim_start_matches('/'))
        .map(|relative| {
            if relative.is_empty() {
                DEFAULT_SCOPED_ROOT.to_string()
            } else {
                format!("{DEFAULT_SCOPED_ROOT}/{relative}")
            }
        })
}

fn strip_leading_current_dir_segments(mut path: &str) -> &str {
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped;
    }
    path
}

/// Resolve a caller-supplied path through the mount view, enforcing the same
/// sensitivity and permission gates as the production coding dispatch.
pub(crate) fn resolve_input_path(
    ctx: &CodingEngineContext,
    path: &str,
    operation: ironclaw_filesystem::FilesystemOperation,
) -> Result<ResolvedCodingPath, CodingEngineError> {
    use ironclaw_safety::sensitive_paths::is_sensitive_path_str;

    let scoped_path = ctx
        .mounts
        .scoped_path(scoped_path_input(path))
        .map_err(|error| {
            tracing::debug!(%error, "pinned scoped path resolution failed");
            coding_error(
                CodingEngineErrorKind::PathResolution,
                format!("{path} is not under an available scoped root"),
            )
        })?;
    if is_sensitive_path_str(scoped_path.as_str()) {
        return Err(coding_error(
            CodingEngineErrorKind::PathResolution,
            format!("{path} resolves to a sensitive path"),
        ));
    }
    let (virtual_path, grant) = ctx
        .mounts
        .resolve_with_grant(&scoped_path)
        .map_err(|error| {
            tracing::debug!(%error, "pinned mount resolution failed");
            coding_error(
                CodingEngineErrorKind::PathResolution,
                format!("{path} does not resolve inside an available scoped root"),
            )
        })?;
    if is_sensitive_path_str(virtual_path.as_str()) {
        return Err(coding_error(
            CodingEngineErrorKind::PathResolution,
            format!("{path} resolves to a sensitive path"),
        ));
    }
    if !super::paths::operation_allowed(&grant.permissions, operation) {
        return Err(coding_error(
            CodingEngineErrorKind::PathResolution,
            format!("the mount for {path} does not permit this operation"),
        ));
    }
    Ok(ResolvedCodingPath {
        virtual_path,
        grant: grant.clone(),
    })
}

/// Display path of `candidate` relative to the workspace mount root, matching
/// pinned source's `formatPathRelativeToCwd` shape (`.` for the root itself).
pub(crate) fn display_path(
    root: &ironclaw_host_api::path::VirtualPath,
    candidate: &ironclaw_host_api::path::VirtualPath,
) -> String {
    let target = root.as_str().trim_end_matches('/');
    let raw = candidate.as_str();
    if raw == target {
        return ".".to_string();
    }
    raw.strip_prefix(&format!("{target}/"))
        .unwrap_or(raw)
        .to_string()
}

/// The virtual mount root the workspace alias resolves to, when the mount
/// view authorizes it. Engines render display paths relative to this root;
/// callers propagate `None` as a path-resolution failure.
pub(crate) fn workspace_virtual_root(
    ctx: &CodingEngineContext,
) -> Option<ironclaw_host_api::path::VirtualPath> {
    let scoped = match ctx.mounts.scoped_path(DEFAULT_SCOPED_ROOT) {
        Ok(scoped) => scoped,
        Err(error) => {
            tracing::debug!(%error, "pinned workspace mount root scoped-path lookup failed");
            return None;
        }
    };
    match ctx.mounts.resolve_with_grant(&scoped) {
        Ok((virtual_path, _)) => Some(virtual_path),
        Err(error) => {
            tracing::debug!(%error, "pinned workspace mount root resolution failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::first_edit_target_path;

    #[test]
    fn first_edit_target_path_uses_first_hashline_section() {
        let input = serde_json::json!({
            "input": "*** Begin Patch\n[second/project.rs#A1B2]\nPUT 1.=1:\n+two\n[first/project.rs#C3D4]\nPUT 1.=1:\n+one\n*** End Patch\n"
        });

        assert_eq!(
            first_edit_target_path(&input).as_deref(),
            Some("/workspace/second/project.rs")
        );
    }
}
