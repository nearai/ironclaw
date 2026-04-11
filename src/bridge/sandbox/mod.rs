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
//! # Scope
//!
//! - [`maybe_intercept`] — given an action name, params, and a project's
//!   mount table, decide whether to handle the call via the mount backend
//!   and produce a `Result<String>` matching what `execute_tool_with_safety`
//!   would return. Handles `file_read`, `file_write`, `list_dir`,
//!   `apply_patch`, and `shell` for paths under `/project/`.
//!
//! - The five sandbox tool names live in [`SANDBOX_TOOL_NAMES`].
//!
//! - [`ProjectSandboxManager`] manages per-project Docker containers and
//!   their lifecycle. [`DockerTransport`] speaks NDJSON to the in-container
//!   `sandbox_daemon`.
//!
//! [`EffectBridgeAdapter`]: super::EffectBridgeAdapter
//! [`WorkspaceMounts`]: ironclaw_engine::WorkspaceMounts

mod containerized_backend;
mod containerized_factory;
mod docker_transport;
mod filesystem_factory;
mod intercept;
mod lifecycle;
mod manager;
pub mod protocol;
mod transport;
pub mod workspace_path;

/// Returns whether the per-project sandbox is enabled.
///
/// Checks `SANDBOX_ENABLED` (same env var as the v1 sandbox) so a single
/// flag governs sandbox behavior regardless of engine version. Also
/// accepts `ENGINE_V2_SANDBOX` as an override for environments that
/// want v1 sandbox off but v2 sandbox on (transitional). Either being
/// truthy (`1`/`true`/`TRUE`/`yes`/`on`) is sufficient.
pub fn engine_v2_sandbox_enabled() -> bool {
    parse_engine_v2_sandbox(
        std::env::var("SANDBOX_ENABLED").ok().as_deref(),
        std::env::var("ENGINE_V2_SANDBOX").ok().as_deref(),
    )
}

fn parse_engine_v2_sandbox(sandbox_enabled: Option<&str>, engine_v2_sandbox: Option<&str>) -> bool {
    is_truthy(sandbox_enabled) || is_truthy(engine_v2_sandbox)
}

fn is_truthy(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true" | "TRUE" | "yes" | "on"))
}

pub use containerized_backend::ContainerizedFilesystemBackend;
pub use containerized_factory::ContainerizedMountFactory;
pub use docker_transport::{DEFAULT_CALL_TIMEOUT, DockerTransport};
pub use filesystem_factory::{FilesystemMountFactory, PROJECT_MOUNT_PREFIX, ProjectPathResolver};
pub use intercept::{InterceptOutcome, SANDBOX_TOOL_NAMES, maybe_intercept};
pub use lifecycle::{DEFAULT_IMAGE, container_name_for, sandbox_image};
pub use manager::ProjectSandboxManager;
pub use transport::SandboxTransport;
pub use workspace_path::{
    PROJECTS_SUBDIR, default_project_workspace_path, ensure_project_workspace_dir,
    project_workspace_path,
};

#[cfg(test)]
mod env_tests {
    use super::parse_engine_v2_sandbox;

    #[test]
    fn sandbox_enabled_truthy_values() {
        for v in ["1", "true", "TRUE", "yes", "on"] {
            assert!(
                parse_engine_v2_sandbox(Some(v), None),
                "SANDBOX_ENABLED='{v}' should enable sandbox"
            );
        }
    }

    #[test]
    fn engine_v2_sandbox_truthy_values() {
        for v in ["1", "true", "TRUE", "yes", "on"] {
            assert!(
                parse_engine_v2_sandbox(None, Some(v)),
                "ENGINE_V2_SANDBOX='{v}' should enable sandbox"
            );
        }
    }

    #[test]
    fn either_flag_suffices() {
        assert!(parse_engine_v2_sandbox(Some("true"), None));
        assert!(parse_engine_v2_sandbox(None, Some("1")));
        assert!(parse_engine_v2_sandbox(Some("1"), Some("true")));
    }

    #[test]
    fn falsy_or_unset_disables() {
        for v in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("no"),
            Some("off"),
        ] {
            assert!(
                !parse_engine_v2_sandbox(v, None),
                "expected {v:?} to disable sandbox"
            );
        }
        assert!(!parse_engine_v2_sandbox(None, None));
    }
}
