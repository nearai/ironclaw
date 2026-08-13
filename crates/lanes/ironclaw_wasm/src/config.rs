use std::time::Duration;

use crate::wasm_sandbox_core::SandboxLimits;

/// WIT package version supported by the Reborn WASM tool runtime.
///
/// ## ABI compatibility
///
/// The 0.3 → 0.4 transition is a **breaking change**: the `near:agent@0.3` ABI
/// added Nostr host functions (`nostr-sign-event`, `nostr-publish-event`,
/// `nostr-subscribe-events`) that are not present in any 0.3.x version. All
/// WASM components built against the 0.3 ABI **must be recompiled** against
/// 0.4 before they can run on this runtime — loading a 0.3 component will
/// fail at link/validation time because the host no longer supplies a matching
/// 0.3 world.
///
/// ### Migration path for tool authors
///
/// 1. Update your `wit-bindgen` path to point at the 0.4 `tool.wit` (shipped
///    in this crate under `wit/`).
/// 2. Regenerate bindings (`cargo build` will do this automatically if your
///    `wit-bindgen::generate!` call references the same WIT source).
/// 3. Rebuild the component. No source changes are needed unless you want to
///    use the new Nostr functions.
/// 4. Deploy the rebuilt component.
pub const WIT_TOOL_VERSION: &str = "0.4.0";

/// Source text of the canonical tool ABI, `wit/tool.wit`, which this crate
/// owns.
///
/// Exported because four call sites — two in this crate's own integration
/// tests, two in `ironclaw_host_runtime` — build component fixtures by
/// re-parsing the same WIT with `wit_parser::Resolve::push_str`. Before the
/// directory moved inside this crate they each reached the repo root with
/// their own `include_str!`; keeping that shape afterwards would have turned
/// the two `ironclaw_host_runtime` sites into cross-crate compile-time
/// reach-ins, which is exactly what PROPOSAL §11.2.7 forbids. One `include_str!`
/// at the owner plus a public const is the same bytes with none of that.
pub const TOOL_WIT: &str = include_str!("../wit/tool.wit");

pub(crate) const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const DEFAULT_HTTP_TIMEOUT_MS: u32 = 30_000;
pub(crate) const MAX_LOGS_PER_EXECUTION: usize = 1_000;
pub(crate) const MAX_LOG_MESSAGE_BYTES: usize = 4 * 1024;

/// Configuration for the Reborn WIT tool runtime.
///
/// Per-execution resource limits use the shared
/// [`crate::wasm_sandbox_core::SandboxLimits`] (identical
/// `memory_bytes`/`fuel`/`timeout` triple and defaults).
#[derive(Debug, Clone, Default)]
pub struct WitToolRuntimeConfig {
    pub default_limits: SandboxLimits,
}

impl WitToolRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            default_limits: SandboxLimits::default()
                .with_memory_bytes(1024 * 1024)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
        }
    }

    /// Like `for_testing` but with a custom memory limit (in bytes).
    pub fn for_testing_with_memory(memory_bytes: u64) -> Self {
        Self {
            default_limits: SandboxLimits::default()
                .with_memory_bytes(memory_bytes)
                .with_fuel(100_000)
                .with_timeout(Duration::from_secs(5)),
        }
    }
}
