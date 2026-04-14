//! Resource limits data types for WASM sandbox execution.
//!
//! The pure-data types live here so callers can depend on them without the
//! `wasm-sandbox` feature. The `WasmResourceLimiter` impl (which depends on
//! wasmtime) stays in `src/tools/wasm/limits.rs`.

use std::time::Duration;

/// Default memory limit: 10 MB (conservative for untrusted code).
pub const DEFAULT_MEMORY_LIMIT: u64 = 10 * 1024 * 1024;

/// Default fuel limit: 100 million instructions.
pub const DEFAULT_FUEL_LIMIT: u64 = 100_000_000;

/// Default execution timeout: 60 seconds.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Resource limits for a single WASM execution.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum memory in bytes.
    pub memory_bytes: u64,
    /// Maximum fuel (instruction count).
    pub fuel: u64,
    /// Maximum wall-clock execution time.
    pub timeout: Duration,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_LIMIT,
            fuel: DEFAULT_FUEL_LIMIT,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl ResourceLimits {
    /// Create limits with custom memory.
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.memory_bytes = bytes;
        self
    }

    /// Create limits with custom fuel.
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    /// Create limits with custom timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Configuration for fuel metering.
#[derive(Debug, Clone)]
pub struct FuelConfig {
    /// Initial fuel to provide.
    pub initial_fuel: u64,
    /// Whether to enable fuel consumption.
    pub enabled: bool,
}

impl Default for FuelConfig {
    fn default() -> Self {
        Self {
            initial_fuel: DEFAULT_FUEL_LIMIT,
            enabled: true,
        }
    }
}

impl FuelConfig {
    /// Create a disabled fuel config (no CPU limits).
    pub fn disabled() -> Self {
        Self {
            initial_fuel: 0,
            enabled: false,
        }
    }

    /// Create a fuel config with a custom limit.
    pub fn with_limit(fuel: u64) -> Self {
        Self {
            initial_fuel: fuel,
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_limits() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.memory_bytes, DEFAULT_MEMORY_LIMIT);
        assert_eq!(limits.fuel, DEFAULT_FUEL_LIMIT);
        assert_eq!(limits.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_limits_builder() {
        let limits = ResourceLimits::default()
            .with_memory(5 * 1024 * 1024)
            .with_fuel(1_000_000)
            .with_timeout(std::time::Duration::from_secs(30));

        assert_eq!(limits.memory_bytes, 5 * 1024 * 1024);
        assert_eq!(limits.fuel, 1_000_000);
        assert_eq!(limits.timeout, std::time::Duration::from_secs(30));
    }

    #[test]
    fn test_fuel_config() {
        let config = FuelConfig::default();
        assert!(config.enabled);
        assert_eq!(config.initial_fuel, DEFAULT_FUEL_LIMIT);

        let disabled = FuelConfig::disabled();
        assert!(!disabled.enabled);

        let custom = FuelConfig::with_limit(5_000_000);
        assert!(custom.enabled);
        assert_eq!(custom.initial_fuel, 5_000_000);
    }
}
