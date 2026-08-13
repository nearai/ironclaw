use std::time::SystemTime;

use ironclaw_host_api::{
    mount::MountGrant,
    path::{ScopedPath, VirtualPath},
};

#[derive(Debug, Clone)]
pub(super) struct ResolvedPath {
    pub(super) scoped_path: ScopedPath,
    pub(super) virtual_path: VirtualPath,
    pub(super) grant: MountGrant,
}

impl ResolvedPath {
    /// Whether this resolution IS its grant's mount root.
    ///
    /// A mount root the caller is authorized for exists by definition: it is
    /// the namespace their grant names, not a path they chose. A per-caller
    /// workspace root (`tenants/{tenant}/users/{user}`) does not exist on the
    /// backend until the first write, so reads of the root itself must behave
    /// as an empty directory rather than `NotFound`. Deeper paths keep
    /// reporting `NotFound`.
    pub(super) fn is_mount_root(&self) -> bool {
        self.virtual_path == self.grant.target
    }
}

#[derive(Debug)]
pub(super) struct ListEntry {
    pub(super) display: String,
    pub(super) is_dir: bool,
}

#[derive(Debug)]
pub(super) struct GrepFileResult {
    pub(super) relative: String,
    pub(super) modified: Option<SystemTime>,
    pub(super) count: usize,
    pub(super) lines: Vec<GrepLine>,
}

#[derive(Debug)]
pub(super) struct GrepLine {
    pub(super) number: usize,
    pub(super) text: String,
    pub(super) is_match: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileEncoding {
    Utf8,
    Utf16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatchMethod {
    Exact,
    FuzzyNormalization,
}

impl MatchMethod {
    pub(super) fn as_wire_name(self) -> &'static str {
        match self {
            MatchMethod::Exact => "Exact",
            MatchMethod::FuzzyNormalization => "FuzzyNormalization",
        }
    }
}
