use std::time::Duration;

use ironclaw_wasm_sandbox_core::SandboxLimits;

/// WIT package version supported by the Reborn WASM tool runtime.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

pub(crate) const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const DEFAULT_HTTP_TIMEOUT_MS: u32 = 30_000;
pub(crate) const MAX_LOGS_PER_EXECUTION: usize = 1_000;
pub(crate) const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

/// Configuration for the Reborn WIT tool runtime.
///
/// Per-execution resource limits use the shared
/// [`ironclaw_wasm_sandbox_core::SandboxLimits`] (identical
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
