use std::time::Duration;

use crate::wasm_sandbox_core::SandboxLimits;

/// WIT package version supported by the Reborn WASM tool runtime.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

/// Source text of the canonical tool ABI, `wit/tool.wit`, which this crate
/// owns.
///
/// Exported because four call sites — two in this crate's own integration
/// tests, two in `ironclaw_host_runtime` — build component fixtures by
/// re-parsing the same WIT with `wit_parser::Resolve::push_str`. Before the
/// directory moved inside this crate they each reached the repo root with
/// their own `include_str!`; keeping that shape afterwards would have turned
/// the two `ironclaw_host_runtime` sites into cross-crate compile-time
/// reach-ins, which is exactly what PROPOSAL §11.2.7 forbids. One `include_str!`
/// at the owner plus a public const is the same bytes with none of that.
pub const TOOL_WIT: &str = include_str!("../wit/tool.wit");

/// Maximum raw UTF-8 bytes accepted for one guest-controlled diagnostic.
pub const WASM_DIAGNOSTIC_MAX_BYTES: usize = 4 * 1024;

/// Maximum guest diagnostic entries accepted during one execution.
pub const WASM_DIAGNOSTIC_MAX_ENTRIES_PER_EXECUTION: usize = 1_000;

/// Stable replacement for a guest diagnostic that cannot be exposed safely.
pub const WASM_DIAGNOSTIC_REDACTION_MARKER: &str = "[WASM_DIAGNOSTIC_REDACTED]";

pub(crate) const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const DEFAULT_HTTP_TIMEOUT_MS: u32 = 30_000;

/// Configuration for the Reborn WIT tool runtime.
///
/// Per-execution resource limits use the shared
/// [`crate::wasm_sandbox_core::SandboxLimits`] (identical
/// `memory_bytes`/`fuel`/`timeout` triple and defaults).
#[derive(Debug, Clone, Default)]
pub struct WitToolRuntimeConfig {
    pub default_limits: SandboxLimits,
}

impl WitToolRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            default_limits: SandboxLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
        }
    }
}
