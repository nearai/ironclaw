//! Tool rate limit and discovery configuration shared across crates.
//!
//! `ToolRateLimitConfig` is used both for built-in tool invocation rate
//! limiting and re-exported as `capabilities::RateLimitConfig` for WASM
//! HTTP capability configuration. `ToolDiscoverySummary` describes the
//! curated discovery guidance returned by `tool_info(detail: "summary")`.

use serde::{Deserialize, Serialize};

/// Per-tool rate limit configuration for tool invocations.
///
/// Controls how many times a tool can be invoked per user, per time window.
/// Read-only tools (echo, time, json, file_read, etc.) should NOT be rate limited.
/// Write/external tools (shell, http, file_write, memory_write, create_job) should be.
#[derive(Debug, Clone)]
pub struct ToolRateLimitConfig {
    /// Maximum invocations per minute.
    pub requests_per_minute: u32,
    /// Maximum invocations per hour.
    pub requests_per_hour: u32,
}

impl ToolRateLimitConfig {
    /// Create a config with explicit limits.
    pub fn new(requests_per_minute: u32, requests_per_hour: u32) -> Self {
        Self {
            requests_per_minute,
            requests_per_hour,
        }
    }
}

impl Default for ToolRateLimitConfig {
    /// Default: 60 requests/minute, 1000 requests/hour (generous for WASM HTTP).
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            requests_per_hour: 1000,
        }
    }
}

/// Curated discovery guidance surfaced by `tool_info(detail: "summary")`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolDiscoverySummary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional_requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<serde_json::Value>,
}
