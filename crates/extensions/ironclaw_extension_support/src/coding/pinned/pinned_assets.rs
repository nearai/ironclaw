//! Compile-embedded pinned registration assets (issue #7392 slice 3).
//!
//! The model-visible coding-tool descriptions and pinned input schemas are contract
//! bytes owned by this crate. They are embedded HERE with single-segment
//! `include_str!` paths (inside the owning crate root) and exposed as public
//! constants so downstream crates — `ironclaw_host_runtime`'s
//! test-support-gated coding registration — resolve them through this crate's
//! public API instead of cross-crate reach-ins (see
//! `ironclaw_architecture_tests::reborn_cross_crate_include_scan`, §11.2.7).
//!
//! The `reborn_coding_engines` root test keeps the schemas and fully supported
//! descriptions byte-identical to `tests/fixtures/pinned_coding_contract/`.
//! `read` intentionally uses an IronClaw-specific description because the
//! pinned upstream prompt advertises source kinds that this implementation
//! does not provide yet.

/// Model-visible `read` tool description, narrowed to the source kinds
/// implemented by IronClaw today.
pub const CODING_READ_DESCRIPTION: &str = include_str!("assets/prompts/read.ironclaw.md");
/// Pinned model-visible `write` tool description.
pub const CODING_WRITE_DESCRIPTION: &str = include_str!("assets/prompts/write.md");
/// Pinned model-visible `edit` tool description (the hashline prompt).
pub const CODING_EDIT_DESCRIPTION: &str = include_str!("assets/prompts/hashline.md");
/// Pinned model-visible `glob` tool description.
pub const CODING_GLOB_DESCRIPTION: &str = include_str!("assets/prompts/glob.md");
/// Pinned model-visible `grep` tool description.
pub const CODING_GREP_DESCRIPTION: &str = include_str!("assets/prompts/grep.md");
/// Model-visible `bash` tool description, rendered from the pinned upstream
/// `bash.md` template with IronClaw's surface flags (no `eval`, `hub`,
/// shell builtins, async, or auto-background; no Windows variants).
pub const CODING_BASH_DESCRIPTION: &str = include_str!("assets/prompts/bash.ironclaw.md");

/// `read` input schema, narrowed to the source kinds implemented by IronClaw.
pub const CODING_READ_SCHEMA: &str = include_str!("assets/schemas/read.ironclaw.json");
/// Pinned `write` input schema.
pub const CODING_WRITE_SCHEMA: &str = include_str!("assets/schemas/write.json");
/// Pinned `edit` input schema.
pub const CODING_EDIT_SCHEMA: &str = include_str!("assets/schemas/edit.json");
/// Pinned `glob` input schema.
pub const CODING_GLOB_SCHEMA: &str = include_str!("assets/schemas/glob.json");
/// Pinned `grep` input schema.
pub const CODING_GREP_SCHEMA: &str = include_str!("assets/schemas/grep.json");
/// `bash` input schema, narrowed to the fields the IronClaw process port
/// supports (`command`/`env`/`timeout`/`cwd`; no `pty` or `async`).
pub const CODING_BASH_SCHEMA: &str = include_str!("assets/schemas/bash.ironclaw.json");
