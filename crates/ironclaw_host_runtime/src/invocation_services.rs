//! Host API service bindings resolved for one invocation.
//!
//! Capability manifests remain the declaration layer for required host APIs.
//! This module contains the concrete binding layer: after policy/planning and
//! run-profile resolution approve an invocation, composition supplies these
//! services to runtime adapters. First-party handlers consume the Rust traits
//! directly; Script, WASM, MCP, and command-backed adapters should adapt the same
//! bindings into their runtime-specific host APIs rather than resolve placement
//! independently.

use std::{fmt, sync::Arc};

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::RuntimeHttpEgress;

use crate::RuntimeProcessPort;

/// Concrete host API bindings for an already-authorized invocation.
///
/// This type is intentionally runtime-agnostic. It represents the approved
/// host API services for a run profile, not a new capability taxonomy.
#[derive(Clone)]
#[non_exhaustive]
pub struct InvocationServices {
    pub filesystem: Arc<dyn RootFilesystem>,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub process: Arc<dyn RuntimeProcessPort>,
}

impl fmt::Debug for InvocationServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationServices")
            .field("filesystem", &"<root filesystem>")
            .field(
                "runtime_http_egress",
                &self
                    .runtime_http_egress
                    .as_ref()
                    .map(|_| "<runtime http egress>"),
            )
            .field("process", &"<runtime process port>")
            .finish()
    }
}
