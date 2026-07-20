use std::time::Duration;

pub use ironclaw_wasm::wasm_sandbox_core::SandboxLimits as ProductAdapterComponentLimits;

pub const PRODUCT_ADAPTER_WIT_VERSION: &str = "0.1.0";

pub(crate) const DEFAULT_MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) const MAX_LOGS_PER_EXECUTION: usize = 1_000;
pub(crate) const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone)]
pub struct ProductAdapterComponentRuntimeConfig {
    pub default_limits: ProductAdapterComponentLimits,
    pub max_component_bytes: usize,
}

impl Default for ProductAdapterComponentRuntimeConfig {
    fn default() -> Self {
        Self {
            default_limits: ProductAdapterComponentLimits::default(),
            max_component_bytes: DEFAULT_MAX_COMPONENT_BYTES,
        }
    }
}

impl ProductAdapterComponentRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            default_limits: ProductAdapterComponentLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
            max_component_bytes: 512 * 1024,
        }
    }
}
