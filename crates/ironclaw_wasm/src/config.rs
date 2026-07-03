use std::time::Duration;

/// WIT package version supported by the Reborn WASM tool runtime.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

pub(crate) const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const DEFAULT_HTTP_TIMEOUT_MS: u32 = 30_000;
pub(crate) const MAX_LOGS_PER_EXECUTION: usize = 1_000;
pub(crate) const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

const DEFAULT_MEMORY_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_FUEL: u64 = 500_000_000;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Resource limits for one WIT tool execution.
#[derive(Debug, Clone)]
pub struct WitToolLimits {
    pub memory_bytes: u64,
    pub fuel: u64,
    pub timeout: Duration,
}

impl Default for WitToolLimits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_BYTES,
            fuel: DEFAULT_FUEL,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl WitToolLimits {
    pub fn with_memory_bytes(mut self, memory_bytes: u64) -> Self {
        self.memory_bytes = memory_bytes;
        self
    }

    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Configuration for the Reborn WIT tool runtime.
#[derive(Debug, Clone, Default)]
pub struct WitToolRuntimeConfig {
    pub default_limits: WitToolLimits,
}

impl WitToolRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            default_limits: WitToolLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
        }
    }
}
