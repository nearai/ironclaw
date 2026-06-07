use std::time::{Duration, Instant};

use wasmtime::component::Linker;
use wasmtime::{Config, Engine, ResourceLimiter, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

pub use wasmtime_wasi::{WasiCtxView as MinimalWasiCtxView, WasiView as MinimalWasiView};

/// Runtime-specific store data exposes the shared v1-style limiter to Wasmtime.
pub trait SandboxStoreData {
    fn sandbox_limiter(&mut self) -> &mut WasmResourceLimiter;
}

/// v1-compatible epoch tick interval used as backup timeout mechanism.
pub const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);

const DEFAULT_MEMORY_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_FUEL: u64 = 500_000_000;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct SandboxLimits {
    pub memory_bytes: u64,
    pub fuel: u64,
    pub timeout: Duration,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_BYTES,
            fuel: DEFAULT_FUEL,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl SandboxLimits {
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

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to create WASM engine: {0}")]
    EngineCreationFailed(String),
    #[error("failed to configure WASM store: {0}")]
    StoreConfiguration(String),
    #[error("failed to configure WASM linker: {0}")]
    LinkerConfiguration(String),
}

/// Store-owned v1-style sandbox state: resource limiter plus minimal WASI p2.
pub struct SandboxStoreCore {
    limiter: WasmResourceLimiter,
    wasi: WasiCtx,
    table: ResourceTable,
    deadline: Option<Instant>,
}

impl SandboxStoreCore {
    pub fn new(memory_limit: u64, timeout: Duration) -> Self {
        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            deadline: Instant::now().checked_add(timeout),
        }
    }

    pub fn limiter_mut(&mut self) -> &mut WasmResourceLimiter {
        &mut self.limiter
    }

    pub fn deadline_exceeded(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiView for SandboxStoreCore {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        self.ctx()
    }
}

/// Wasmtime ResourceLimiter implementation from the v1 WASM sandbox.
///
/// This intentionally mirrors the Reborn v1 behavior: memory checks track
/// aggregate growth across component-model memories while still permitting
/// multiple internal instances/memories.
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
        self.max_memories as usize
    }
}

pub fn component_engine(thread_name: impl Into<String>) -> Result<Engine, SandboxError> {
    let mut config = Config::new();
    configure_component_engine(&mut config);
    let engine = Engine::new(&config)
        .map_err(|error| SandboxError::EngineCreationFailed(error.to_string()))?;
    spawn_epoch_ticker(&engine, thread_name)?;
    Ok(engine)
}

pub fn configure_component_engine(config: &mut Config) {
    config.consume_fuel(true);
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    config.wasm_threads(false);
    config.debug_info(false);
}

/// Spawn the per-engine epoch ticker.
///
/// The ticker holds a `Weak<Engine>` reference rather than a strong clone so
/// that, once every owned `Engine` clone has been dropped by the host, the
/// ticker observes `upgrade() == None`, exits, and the thread joins. Without
/// this, every `component_engine` call would leak both a thread and an
/// `Engine` clone for the lifetime of the process — a real problem for tests
/// and for long-running hosts that rebuild runtimes (config reloads,
/// per-installation runtimes, etc.).
pub fn spawn_epoch_ticker(
    engine: &Engine,
    thread_name: impl Into<String>,
) -> Result<(), SandboxError> {
    let weak = engine.weak();
    std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK_INTERVAL);
                match weak.upgrade() {
                    Some(engine) => engine.increment_epoch(),
                    None => break,
                }
            }
        })
        .map(|_| ())
        .map_err(|error| SandboxError::EngineCreationFailed(error.to_string()))
}

pub fn configure_store<T>(store: &mut Store<T>, limits: &SandboxLimits) -> Result<(), SandboxError>
where
    T: SandboxStoreData + 'static,
{
    store
        .set_fuel(limits.fuel)
        .map_err(|error| SandboxError::StoreConfiguration(error.to_string()))?;
    store.epoch_deadline_trap();
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| data.sandbox_limiter());
    Ok(())
}

pub fn add_minimal_wasi_to_linker<T>(linker: &mut Linker<T>) -> Result<(), SandboxError>
where
    T: WasiView,
{
    wasmtime_wasi::p2::add_to_linker_sync(linker)
        .map_err(|error| SandboxError::LinkerConfiguration(error.to_string()))
}

pub fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use wasmtime::ResourceLimiter;

    use super::{SandboxStoreCore, WasmResourceLimiter};

    #[test]
    fn minimal_wasi_store_starts_without_deadline_exceeded() {
        let core = SandboxStoreCore::new(1024 * 1024, std::time::Duration::from_secs(1));
        assert!(!core.deadline_exceeded());
    }

    #[test]
    fn limiter_tracks_aggregate_growth_across_memories() {
        let mut limiter = WasmResourceLimiter::new(128 * 1024);
        assert!(limiter.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(limiter.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(!limiter.memory_growing(0, 64 * 1024, None).unwrap());
    }

    #[test]
    fn limiter_allows_component_model_internal_resources_like_v1() {
        let limiter = WasmResourceLimiter::new(1024);
        assert_eq!(limiter.instances(), 10);
        assert_eq!(limiter.tables(), 10);
        assert_eq!(limiter.memories(), 10);
    }

    /// Regression: the epoch ticker thread must observe `Weak::upgrade() ==
    /// None` and exit after every `Engine` clone is dropped. Without this,
    /// each `component_engine` call would permanently leak a thread + an
    /// `Engine` clone, which is visible immediately in test runs and matters
    /// even more in long-running hosts that rebuild runtimes on config
    /// reload or per-installation.
    #[test]
    fn epoch_ticker_exits_when_engine_is_dropped() {
        use super::{EPOCH_TICK_INTERVAL, component_engine};
        use std::thread;
        use std::time::Duration;

        let engine =
            component_engine("sandbox-core-ticker-drop-test").expect("engine should construct");
        // Take a weak handle so we can observe when every owned `Engine`
        // clone has been dropped. We piggy-back on wasmtime's own
        // `Engine::weak()` for this.
        let engine_weak = engine.weak();
        // Drop the caller's strong reference. If the ticker is still
        // holding a clone (the pre-fix bug), `upgrade()` will keep
        // succeeding forever.
        drop(engine);

        // The ticker wakes at most once per EPOCH_TICK_INTERVAL. Give it a
        // few intervals of slack for scheduler jitter before declaring the
        // shutdown broken.
        let mut released = false;
        for _ in 0..10 {
            thread::sleep(EPOCH_TICK_INTERVAL + Duration::from_millis(100));
            if engine_weak.upgrade().is_none() {
                released = true;
                break;
            }
        }
        assert!(
            released,
            "epoch ticker still holding an Engine clone after >10 tick intervals; the Weak<Engine> shutdown path is broken"
        );
    }
}
