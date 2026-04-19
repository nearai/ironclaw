//! Workspace mount-table abstraction.
//!
//! Defines the [`MountBackend`] trait — a uniform interface for executing
//! filesystem and shell operations against a storage backend — and a small
//! [`WorkspaceMounts`] registry that resolves agent-facing paths (like
//! `/project/foo.txt`) to the backend that owns them.
//!
//! This is a deliberately small subset of the unified Workspace VFS proposed
//! in nearai/ironclaw#1894. Engine v2's per-project sandbox needs the
//! abstraction so that the same agent-facing path scheme works whether the
//! `/project/` mount is served by the host filesystem (default) or by a
//! containerized backend that dispatches into a per-project sandbox container
//! over JSON-RPC.
//!
//! Two backends ship in this crate:
//!
//! - [`FilesystemBackend`] — passthrough to a real filesystem rooted at a
//!   host path. Used by the bridge when no sandbox is configured.
//! - The bridge's `ContainerizedFilesystemBackend` (separate module, see
//!   `src/bridge/sandbox/`) — JSON-RPC into a per-project container. Lives
//!   in the host crate because it needs Docker.
//!
//! The trait itself stays in the engine crate so both backends — and any
//! future backends added by nearai/ironclaw#1894 — can implement the same
//! interface without depending on host infrastructure.

pub mod filesystem;
pub mod mount;
pub mod registry;

pub use filesystem::FilesystemBackend;
pub use mount::{DirEntry, EntryKind, MountBackend, MountError, ShellOutput};
pub use registry::{ProjectMountFactory, ProjectMounts, WorkspaceMounts};
