use std::time::Duration;

pub use ironclaw_wasm_sandbox_core::SandboxLimits as ProductAdapterComponentLimits;

pub const PRODUCT_ADAPTER_WIT_VERSION: &str = "0.1.0";

pub(crate) const MAX_LOGS_PER_EXECUTION: usize = 1_000;
pub(crate) const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Default)]
pub struct ProductAdapterComponentRuntimeConfig {
    pub default_limits: ProductAdapterComponentLimits,
}

impl ProductAdapterComponentRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            default_limits: ProductAdapterComponentLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
        }
    }
}
