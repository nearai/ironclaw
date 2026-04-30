use wasmtime::ResourceLimiter;

#[derive(Debug)]
pub(crate) struct WasmResourceLimiter {
    memory_limit: u64,
    max_tables: u32,
    max_instances: u32,
    max_memories: u32,
}

impl WasmResourceLimiter {
    pub(crate) fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            max_tables: 10,
            max_instances: 10,
            max_memories: 1,
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
        let desired = desired as u64;
        if desired > self.memory_limit {
            tracing::warn!(
                current,
                desired,
                limit = self.memory_limit,
                "WASM memory growth denied"
            );
            return Ok(false);
        }
        Ok(true)
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
    fn memories_limit_fails_closed_to_single_canonical_memory() {
        let limiter = WasmResourceLimiter::new(1024);
        assert_eq!(limiter.instances(), 10);
        assert_eq!(limiter.tables(), 10);
        assert_eq!(limiter.memories(), 1);
    }
}
