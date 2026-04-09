//! Engine v2 per-project sandbox bridge.
//!
//! This submodule is the **host-side glue** between [`EffectBridgeAdapter`]
//! and the engine's [`WorkspaceMounts`] abstraction. It is the place where
//! the bridge decides — for any given tool call — whether to dispatch into
//! a sandbox backend (filesystem or, eventually, containerized) or fall
//! through to direct host tool execution.
//!
//! See `docs/plans/2026-03-20-engine-v2-architecture.md` (Phase 8) and the
//! plan at `~/.claude/plans/effervescent-munching-moore.md` for the full
//! design rationale, including the cross-reference with nearai/ironclaw#1894.
//!
//! # Phase 1 scope
//!
//! Only the wiring is here:
//!
//! - [`maybe_intercept`] — given an action name, params, and a project's
//!   mount table, decide whether to handle the call via the mount backend
//!   and produce a `Result<String>` matching what `execute_tool_with_safety`
//!   would return. Currently handles `file_read`, `file_write`, and
//!   `list_dir` for paths under `/project/`. `apply_patch` and `shell`
//!   intentionally fall through to host execution until the
//!   `ContainerizedFilesystemBackend` lands in Phase 5.
//!
//! - The five sandbox tool names live in [`SANDBOX_TOOL_NAMES`].
//!
//! Phase 5 will add `ProjectSandboxManager`, `ContainerHandle`, and the
//! JSON-RPC dispatch.
//!
//! [`EffectBridgeAdapter`]: super::EffectBridgeAdapter
//! [`WorkspaceMounts`]: ironclaw_engine::WorkspaceMounts

mod intercept;

pub use intercept::{InterceptOutcome, SANDBOX_TOOL_NAMES, maybe_intercept};
