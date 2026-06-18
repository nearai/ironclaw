use glob::MatchOptions;

pub(super) const MAX_READ_SIZE: u64 = 10 * 1024 * 1024;
pub(super) const DEFAULT_LINE_LIMIT: usize = 2_000;
/// Byte ceiling on a single `read_file` result body, enforced *in addition to*
/// `DEFAULT_LINE_LIMIT` — whichever is hit first wins. The line cap alone lets a
/// file with few but very long lines (logs, wide CSV rows, minified data) dump
/// hundreds of KB into the conversation, which then rides along in every later
/// LLM request and inflates each round-trip's latency (pinchbench timeout
/// bucket). Reads past this budget truncate to complete lines and hand the model
/// a `next_offset` to continue. Mirrors the dual line/byte cap in earendil-works/pi.
pub(super) const DEFAULT_READ_MAX_BYTES: usize = 64 * 1024;
pub(super) const MAX_WRITE_SIZE: usize = 5 * 1024 * 1024;
pub(super) const MAX_PATCH_SIZE: u64 = 10 * 1024 * 1024;
pub(super) const MAX_DIR_ENTRIES: usize = 500;
pub(super) const DEFAULT_MAX_RESULTS: usize = 200;
pub(super) const MAX_OUTPUT_SIZE: usize = 64 * 1024;
pub(super) const DEFAULT_HEAD_LIMIT: usize = 250;
pub(super) const GREP_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const MAX_VISITED_ENTRIES: usize = 50_000;
pub(super) const DEFAULT_SCOPED_ROOT: &str = "/workspace";

pub(super) const WORKSPACE_FILES: &[&str] = &[
    "HEARTBEAT.md",
    "MEMORY.md",
    "IDENTITY.md",
    "SOUL.md",
    "AGENTS.md",
    "USER.md",
];

pub(super) const GLOB_MATCH_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

pub(super) const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
];
