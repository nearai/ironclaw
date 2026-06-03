//! Shared test-only env-var harness for `runtime::tests` and
//! `runtime::trigger_poller::tests`. Both modules read or mutate the
//! `IRONCLAW_TRIGGER_POLLER_*` env vars; without a single lock + single
//! `EnvGuard` they would race in the same test binary.
//!
//! Not exposed outside `#[cfg(test)]`.

use std::sync::{LazyLock, Mutex, MutexGuard};

/// Serializes every test that touches `IRONCLAW_TRIGGER_*` env vars so
/// they cannot race each other. cargo test runs tests in parallel by
/// default; without this each env-mutating test would observe sibling
/// tests' mutations. Held for the whole body of every env-touching test.
static TRIGGER_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(super) fn lock_trigger_env() -> MutexGuard<'static, ()> {
    TRIGGER_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// RAII guard that snapshots an env var on construction and restores it
/// on drop. Restores on panic too (Drop runs during unwind), which the
/// manual snapshot/restore pattern does not. Tests install this guard so
/// other tests running in parallel cannot observe their mutations.
pub(super) struct EnvGuard {
    key: &'static str,
    prior: Option<String>,
}

impl EnvGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        let prior = std::env::var(key).ok();
        // SAFETY: env mutation is process-global; restore on Drop covers
        // panic unwind. Callers must hold `lock_trigger_env()` for the
        // life of this guard to serialise against sibling test threads.
        unsafe { std::env::set_var(key, value) };
        Self { key, prior }
    }

    pub(super) fn clear(key: &'static str) -> Self {
        let prior = std::env::var(key).ok();
        // SAFETY: see EnvGuard::set.
        unsafe { std::env::remove_var(key) };
        Self { key, prior }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.prior.take() {
            // SAFETY: see EnvGuard::set.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            // SAFETY: see EnvGuard::set.
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}
