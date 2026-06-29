//! Provider-behavior configuration for the mem0 adapter.
//!
//! The mem0 base URL, API key, and TLS belong to the [`crate::Mem0Transport`]
//! implementation, not here. This struct carries only the small amount of
//! mem0-side behavior the provider itself needs to shape its requests.

use serde::{Deserialize, Serialize};

/// Tunables for the mem0 provider's request shaping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Mem0Config {
    /// Optional mem0 `app_id` stamped onto every add/search so memories from
    /// this IronClaw deployment are partitioned from other apps in the same
    /// mem0 organization. `None` omits the field entirely.
    #[serde(default)]
    pub app_id: Option<String>,
}

impl Mem0Config {
    /// Config with no `app_id` partitioning.
    pub fn new() -> Self {
        Self { app_id: None }
    }

    /// Set the mem0 `app_id` partition for this deployment.
    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }
}
