//! Resource limits for WASM sandbox execution.
//!
//! Pure data types (ResourceLimits, FuelConfig, DEFAULT_*) live in
//! `ironclaw_common::limits`. The wasmtime-specific `WasmResourceLimiter`
//! impl stays here behind the `wasm-sandbox` feature.

pub use ironclaw_common::limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
};

/// Wasmtime ResourceLimiter implementation for enforcing memory limits.
///
/// This is attached to the Store to limit memory growth during execution.
#[cfg(feature = "wasm-sandbox")]
#[derive(Debug)]
pub struct WasmResourceLimiter {
    /// Maximum memory allowed.
    memory_limit: u64,
    /// Current memory usage (tracked across all memories).
    memory_used: u64,
    /// Maximum tables allowed.
    max_tables: u32,
    /// Maximum instances allowed.
    max_instances: u32,
}

#[cfg(feature = "wasm-sandbox")]
impl WasmResourceLimiter {
    /// Create a new limiter with the given memory limit.
    ///
    /// Note: max_instances is set to 10 to accommodate WASM Component Model
    /// which creates multiple internal instances (main component + WASI adapters).
    pub fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            max_tables: 10,
            max_instances: 10, // Component model needs multiple instances for WASI
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

#[cfg(feature = "wasm-sandbox")]
impl wasmtime::ResourceLimiter for WasmResourceLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
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
    ) -> Result<bool, wasmtime::Error> {
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
        // Allow multiple memories for component model with WASI
        self.max_instances as usize
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "wasm-sandbox")]
    use super::WasmResourceLimiter;

    #[cfg(feature = "wasm-sandbox")]
    #[test]
    fn test_resource_limiter_allows_growth_within_limit() {
        use wasmtime::ResourceLimiter;
        let mut limiter = WasmResourceLimiter::new(10 * 1024 * 1024);

        // Growth within limit should be allowed
        let result = limiter.memory_growing(0, 1024 * 1024, None).unwrap();
        assert!(result);
        assert_eq!(limiter.memory_used(), 1024 * 1024);
    }

    #[cfg(feature = "wasm-sandbox")]
    #[test]
    fn test_resource_limiter_denies_growth_beyond_limit() {
        use wasmtime::ResourceLimiter;
        let mut limiter = WasmResourceLimiter::new(10 * 1024 * 1024);

        // Growth beyond limit should be denied
        let result = limiter.memory_growing(0, 20 * 1024 * 1024, None).unwrap();
        assert!(!result);
    }
}
