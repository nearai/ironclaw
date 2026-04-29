pub struct WasmRuntimeConfig {
    pub fuel: u64,
    pub max_output_bytes: u64,
    pub max_memory_bytes: u64,
    pub timeout: Duration,
    pub cache_compiled_modules: bool,
    pub cache_dir: Option<PathBuf>,
    pub epoch_tick_interval: Duration,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            max_output_bytes: DEFAULT_OUTPUT_BYTES,
            max_memory_bytes: DEFAULT_MEMORY_BYTES,
            timeout: DEFAULT_TIMEOUT,
            cache_compiled_modules: true,
            cache_dir: None,
            epoch_tick_interval: DEFAULT_EPOCH_TICK_INTERVAL,
        }
    }
}

impl WasmRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            fuel: 100_000,
            max_output_bytes: 1024,
            max_memory_bytes: 1024 * 1024,
            timeout: Duration::from_secs(5),
            cache_compiled_modules: false,
            cache_dir: None,
            epoch_tick_interval: Duration::from_millis(10),
        }
    }
}
