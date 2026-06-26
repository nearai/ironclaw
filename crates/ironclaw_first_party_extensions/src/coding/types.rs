use std::time::SystemTime;

use ironclaw_host_api::{MountGrant, ScopedPath, VirtualPath};

#[derive(Debug, Clone)]
pub(super) struct ResolvedPath {
    pub(super) scoped_path: ScopedPath,
    pub(super) virtual_path: VirtualPath,
    pub(super) grant: MountGrant,
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
