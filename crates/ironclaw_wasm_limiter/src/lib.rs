//! Shared `wasmtime::ResourceLimiter` used by the tool-WASM runtime
//! (`ironclaw_wasm`), the hook-WASM runtime (`ironclaw_hooks`), and the v1-style
//! sandbox core (`ironclaw_wasm_sandbox_core`).
//!
//! Extracted from `ironclaw_wasm`'s private `limiter.rs` so the hook crate
//! can depend on it through Cargo rather than a `#[path = ...]` file
//! import (henrypark133 must-fix #1 on PR #3634). The consumers
//! enforce identical limits; centralizing the impl prevents drift and
//! makes the dependency edge visible to `cargo check`, `cargo doc`, and
//! architecture-linting tests. `ironclaw_wasm_sandbox_core` previously kept a
//! verbatim copy (plus the `memory_used`/`memory_limit` accessors, now folded
//! in here); it imports this one instead.

use wasmtime::ResourceLimiter;

#[derive(Debug)]
pub struct WasmResourceLimiter {
    memory_limit: u64,
    memory_used: u64,
    pending_memory_growth: u64,
    max_tables: u32,
    max_instances: u32,
    max_memories: u32,
}

impl WasmResourceLimiter {
    pub fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            pending_memory_growth: 0,
            max_tables: 10,
            max_instances: 10,
            max_memories: 10,
        }
    }

    pub fn memory_used(&self) -> u64 {
        self.memory_used
    }

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
    ) -> Result<bool, wasmtime::Error> {
        self.pending_memory_growth = 0;

        let current = current as u64;
        let desired = desired as u64;
        let growth = desired.saturating_sub(current);
        let total_memory = self.memory_used.saturating_add(growth);
        if total_memory > self.memory_limit {
            tracing::warn!(
                current,
                desired,
                growth,
                used = self.memory_used,
                total = total_memory,
                limit = self.memory_limit,
                "WASM memory growth denied"
            );
            return Ok(false);
        }

        self.memory_used = total_memory;
        self.pending_memory_growth = growth;
        tracing::trace!(
            current,
            desired,
            growth,
            used = self.memory_used,
            limit = self.memory_limit,
            "WASM memory growth allowed"
        );
        Ok(true)
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> Result<(), wasmtime::Error> {
        self.memory_used = self.memory_used.saturating_sub(self.pending_memory_growth);
        self.pending_memory_growth = 0;
        tracing::debug!(error = ?error, "WASM memory growth failed after approval");
        Ok(())
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        if desired > 10_000 {
            tracing::warn!(current, desired, "WASM table growth denied");
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
        self.max_memories as usize
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::ResourceLimiter;

    use super::WasmResourceLimiter;

    #[test]
    fn memories_limit_allows_component_model_internal_memories() {
        let limiter = WasmResourceLimiter::new(1024);
        assert_eq!(limiter.instances(), 10);
        assert_eq!(limiter.tables(), 10);
        assert_eq!(limiter.memories(), 10);
    }

    #[test]
    fn memory_growing_tracks_aggregate_growth_across_memories() {
        let mut limiter = WasmResourceLimiter::new(128 * 1024);
        assert!(limiter.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(limiter.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(!limiter.memory_growing(0, 64 * 1024, None).unwrap());
    }

    #[test]
    fn memory_grow_failed_rolls_back_pending_growth() {
        // Test #15 on PR #3634: when wasmtime approves a `memory_growing`
        // request, the limiter stages the growth in `pending_memory_growth`
        // and bumps `memory_used`. If the OS-level grow then fails (for
        // example, mmap returns ENOMEM), wasmtime calls `memory_grow_failed`
        // to let the limiter unwind. Without rollback, subsequent grows
        // would see an inflated `memory_used` and be denied even though no
        // actual memory was committed.
        let mut limiter = WasmResourceLimiter::new(64 * 1024);
        // Stage an approved grow.
        assert!(limiter.memory_growing(0, 32 * 1024, None).unwrap());
        // wasmtime reports the OS-level grow failed.
        let _ = limiter.memory_grow_failed(wasmtime::Error::msg("simulated ENOMEM"));
        // A second grow that would exceed the limit if the first attempt
        // were still counted must now succeed: the first attempt's
        // bookkeeping must have rolled back to zero.
        assert!(
            limiter.memory_growing(0, 64 * 1024, None).unwrap(),
            "memory_grow_failed must release the pending growth so a retry up to the full ceiling can succeed"
        );
    }
}
