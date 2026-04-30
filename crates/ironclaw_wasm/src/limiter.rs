use wasmtime::ResourceLimiter;

#[derive(Debug)]
pub(crate) struct WasmResourceLimiter {
    memory_limit: u64,
    memory_used: u64,
    pending_memory_growth: u64,
    max_tables: u32,
    max_instances: u32,
    max_memories: u32,
}

impl WasmResourceLimiter {
    pub(crate) fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            pending_memory_growth: 0,
            max_tables: 10,
            max_instances: 10,
            max_memories: 10,
        }
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
}
