//! Shared Reborn test-support configuration.

#![allow(dead_code)] // Shared by staged Reborn harness modules as ports opt in.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitConfig {
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for WaitConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(10),
        }
    }
}
