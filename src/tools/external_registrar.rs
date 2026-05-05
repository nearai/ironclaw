//! Plugin seam for downstream binaries that ship additional Rust tools.
//!
//! IronClaw's `register_builtin_tools()` is hardcoded — there's no runtime
//! discovery for Rust-level tools (WASM and MCP have their own discovery
//! paths, but Rust tools have to be compiled into the binary). This trait
//! lets a downstream binary register its own `Tool` implementations into
//! the same [`ToolRegistry`] without patching staging's source.
//!
//! The motivating use case is the private `nearai/ironclaw-abound` fork,
//! which ships Rust tools for the Abound remittance integration that
//! cannot live upstream. Without this seam, the private repo would have
//! to maintain a patch against `src/tools/builtin/mod.rs` and
//! `src/tools/registry.rs` forever, re-resolving conflicts on every
//! upstream change.
//!
//! # Usage
//!
//! ```ignore
//! use std::sync::Arc;
//! use ironclaw::tools::{ExternalToolRegistrar, ToolRegistry};
//!
//! struct MyRegistrar;
//!
//! impl ExternalToolRegistrar for MyRegistrar {
//!     fn register(&self, registry: &Arc<ToolRegistry>) {
//!         if let Some(secrets) = registry.secrets_store() {
//!             registry.register_sync(Arc::new(
//!                 MyCustomTool::new(Arc::clone(secrets))
//!             ));
//!         }
//!     }
//! }
//!
//! // Wire into app startup:
//! let builder = AppBuilder::new(/* ... */)
//!     .with_external_tool_registrar(Arc::new(MyRegistrar));
//! ```
//!
//! # Contract
//!
//! - Called exactly once at startup, after `register_builtin_tools()` and
//!   after all the registry's builder-injected dependencies
//!   (`with_credentials`, `with_database`, `with_http_interceptor`) are in
//!   place. Implementations can rely on those accessors returning populated
//!   values when the host binary configured them.
//! - Must be deterministic and side-effect-free beyond tool registration.
//!   Do not spawn long-lived tasks or hold network connections from inside
//!   `register()`; that belongs in activation, not registration (see
//!   `.claude/rules/lifecycle.md`).
//! - External tool names are not added to [`ToolRegistry::is_protected_tool_name`].
//!   A downstream `Tool::name()` that collides with a protected built-in is
//!   rejected by `register_sync` — keep external names distinct.

use std::sync::Arc;

use crate::tools::ToolRegistry;

/// Implement this trait in a downstream crate to register additional built-in
/// tools alongside IronClaw's. See the module docs for the full contract.
pub trait ExternalToolRegistrar: Send + Sync {
    /// Register any additional tools on the given registry.
    ///
    /// The registry has already had `register_builtin_tools()` called and
    /// all builder-injected dependencies applied, so
    /// [`ToolRegistry::secrets_store`], [`ToolRegistry::credential_registry`],
    /// and [`ToolRegistry::role_lookup`] reflect the configured values.
    fn register(&self, registry: &Arc<ToolRegistry>);
}
