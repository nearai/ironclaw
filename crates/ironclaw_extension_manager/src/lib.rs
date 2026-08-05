//! The product face of extensions (PROPOSAL §6.8.3).
//!
//! `ironclaw_extension_host` owns lifecycle **authority**: it is the only
//! writer of installation state and the active snapshot, it verifies channel
//! ingress, and it runs activation transactions. This crate owns the
//! extension-management **UX semantics** on top of that authority — the
//! agent-callable lifecycle commands and capability handlers, the
//! `LifecycleProductService` port the WebUI routes through, the admin /
//! operator / skill-auto-activate capability handlers, the credential setup
//! views, the `[channel.config]` product service, and the extension hub.
//!
//! The dependency direction is the whole point of the split and is enforced
//! by `reborn_extension_manager_split.rs`: **the manager calls the host, and
//! the host never calls the manager.** That is what lets `extension_host`
//! move below product in WS2's layer flip while this crate stays at
//! `products`, where naming `ironclaw_product` is legal.
//!
//! Nothing here may write installation state directly — every mutation goes
//! through `ironclaw_extension_host::ExtensionLifecycleManager`.

pub mod admin_configuration;
pub mod admin_configuration_capability;
pub mod channel_config_product_service;
pub mod extension_lifecycle_capabilities;
pub mod extension_lifecycle_command;
pub mod ironhub;
pub mod lifecycle_product_service;
pub mod operator_config_capability;
pub mod skill_auto_activate_capability;
mod terminal_render;
pub mod webui_extension_credentials;

// Kept at `test_support/lifecycle.rs` rather than a flat
// `lifecycle_test_support.rs`: the extension-specificity scanner classifies a
// `test_support` *path component* as fixture code, and tests may name concrete
// products (overview §8). Flattening the name would have pushed a fixture into
// the generic-code scan and bought two allowlist entries — a relaxation — for
// nothing.
#[cfg(any(test, feature = "test-support"))]
#[path = "test_support/lifecycle.rs"]
pub mod lifecycle_test_support;

pub use channel_config_product_service::RebornChannelConfigProductService;
pub use lifecycle_product_service::ExtensionHostLifecycleProductService;
