//! Resource limits for WASM sandbox execution.
//!
//! Provides memory and fuel (CPU) limits following NEAR blockchain patterns.

use std::time::Duration;

use wasmtime::ResourceLimiter;

/// Default memory limit: 10 MB (conservative for untrusted code).
pub const DEFAULT_MEMORY_LIMIT: u64 = 10 * 1024 * 1024;

/// Default fuel limit: 10 million instructions.
pub const DEFAULT_FUEL_LIMIT: u64 = 10_000_000;

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

/// Wasmtime ResourceLimiter implementation for enforcing memory limits.
///
/// This is attached to the Store to limit memory growth during execution.
#[derive(Debug)]
pub struct WasmResourceLimiter {
    /// Maximum memory allowed.
    memory_limit: u64,
    /// Current memory usage (tracked across all memories).
    memory_used: u64,
    /// Maximum tables allowed.
    max_tables: u32,
    /// Current table count.
    #[allow(dead_code)] // Reserved for table limit enforcement
    tables_created: u32,
    /// Maximum instances allowed.
    max_instances: u32,
    /// Current instance count.
    #[allow(dead_code)] // Reserved for instance limit enforcement
    instances_created: u32,
}

impl WasmResourceLimiter {
    /// Create a new limiter with the given memory limit.
    pub fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            max_tables: 10,
            tables_created: 0,
            max_instances: 1,
            instances_created: 0,
        }
    }

    /// Get current memory usage.
    pub fn memory_used(&self) -> u64 {
        self.memory_used
    }

    /// Get the memory limit.
    pub fn memory_limit(&self) -> u64 {
        self.memory_limit
    }
}

impl ResourceLimiter for WasmResourceLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let desired_u64 = desired as u64;

        if desired_u64 > self.memory_limit {
            tracing::warn!(
                current = current,
                desired = desired,
                limit = self.memory_limit,
                "WASM memory growth denied: would exceed limit"
            );
            return Ok(false);
        }

        self.memory_used = desired_u64;
        tracing::trace!(
            current = current,
            desired = desired,
            limit = self.memory_limit,
            "WASM memory growth allowed"
        );
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        // Allow reasonable table growth
        if desired > 10_000 {
            tracing::warn!(
                current = current,
                desired = desired,
                "WASM table growth denied: too large"
            );
            return Ok(false);
        }
        Ok(true)
    }

    fn instances(&self) -> usize {
        self.max_instances as usize
    }

    fn tables(&self) -> usize {
        self.max_tables as usize
    }

    fn memories(&self) -> usize {
        // Allow one memory per instance
        self.max_instances as usize
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
    use crate::tools::wasm::limits::{
        DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
        WasmResourceLimiter,
    };
    use wasmtime::ResourceLimiter;

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
    fn test_resource_limiter_allows_growth_within_limit() {
        let mut limiter = WasmResourceLimiter::new(10 * 1024 * 1024);

        // Growth within limit should be allowed
        let result = limiter.memory_growing(0, 1024 * 1024, None).unwrap();
        assert!(result);
        assert_eq!(limiter.memory_used(), 1024 * 1024);
    }

    #[test]
    fn test_resource_limiter_denies_growth_beyond_limit() {
        let mut limiter = WasmResourceLimiter::new(10 * 1024 * 1024);

        // Growth beyond limit should be denied
        let result = limiter.memory_growing(0, 20 * 1024 * 1024, None).unwrap();
        assert!(!result);
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
